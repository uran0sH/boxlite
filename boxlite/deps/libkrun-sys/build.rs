use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn main() {
    // Rebuild if vendored sources change (Linux only)
    println!("cargo:rerun-if-changed=vendor/libkrun");
    println!("cargo:rerun-if-changed=vendor/libkrunfw");

    // Check for stub mode (for CI linting without building)
    // Set LIBKRUN_SYS_STUB=1 to skip building and emit stub link directives
    if env::var("LIBKRUN_SYS_STUB").is_ok() {
        println!("cargo:warning=LIBKRUN_SYS_STUB mode: skipping build, emitting stub directives");
        // Emit minimal link directives that won't actually link anything
        // This allows cargo check/clippy to pass without building libkrun
        println!("cargo:rustc-link-lib=dylib=krun");
        // Use a non-existent path - linking will fail but check/clippy won't try to link
        println!("cargo:libkrun_dir=/nonexistent");
        println!("cargo:libkrunfw_dir=/nonexistent");
        return;
    }

    build();
}

/// Runs a command and panics with a helpful message if it fails.
#[allow(unused)]
fn run_command(cmd: &mut Command, description: &str) {
    let status = cmd
        .status()
        .unwrap_or_else(|e| panic!("Failed to execute {}: {}", description, e));

    if !status.success() {
        panic!("{} failed with exit code: {:?}", description, status.code());
    }
}

/// Checks if a directory contains any library file matching the given prefix.
/// Returns true if a file like "prefix.*.{dylib,so}" exists.
#[allow(unused)]
fn has_library(dir: &Path, prefix: &str) -> bool {
    let extensions = if cfg!(target_os = "macos") {
        vec!["dylib"]
    } else if cfg!(target_os = "linux") {
        vec!["so"]
    } else {
        vec!["dll"]
    };

    dir.read_dir()
        .ok()
        .map(|entries| {
            entries.filter_map(Result::ok).any(|entry| {
                let filename = entry.file_name().to_string_lossy().to_string();
                filename.starts_with(prefix)
                    && extensions
                        .iter()
                        .any(|ext| entry.path().extension().is_some_and(|e| e == *ext))
            })
        })
        .unwrap_or(false)
}

/// Creates a make command with common configuration.
#[allow(unused)]
fn make_command(
    source_dir: &Path,
    install_dir: &Path,
    extra_env: &HashMap<String, String>,
) -> Command {
    let mut cmd = Command::new("make");
    cmd.stdout(Stdio::inherit());
    cmd.stderr(Stdio::inherit());
    cmd.args(["-j", &num_cpus::get().to_string()])
        .arg("MAKEFLAGS=") // Clear MAKEFLAGS to prevent -w flag issues in submakes
        .env("PREFIX", install_dir)
        .current_dir(source_dir);

    // Apply extra environment variables
    for (key, value) in extra_env {
        cmd.env(key, value);
    }

    cmd
}

/// Builds a library using Make with the specified parameters.
#[allow(unused)]
fn build_with_make(
    source_dir: &Path,
    install_dir: &Path,
    lib_name: &str,
    extra_env: HashMap<String, String>,
) {
    println!("cargo:warning=Building {} from source...", lib_name);

    std::fs::create_dir_all(install_dir)
        .unwrap_or_else(|e| panic!("Failed to create install directory: {}", e));

    // Build
    let mut make_cmd = make_command(source_dir, install_dir, &extra_env);
    run_command(&mut make_cmd, &format!("make {}", lib_name));

    // Install
    let mut install_cmd = make_command(source_dir, install_dir, &extra_env);
    install_cmd.arg("install");
    run_command(&mut install_cmd, &format!("make install {}", lib_name));
}

/// Configure linking for libkrun.
///
/// Note: libkrunfw is NOT linked here - it's dlopened by libkrun at runtime.
/// We only expose the library directory so downstream crates can bundle it.
fn configure_linking(libkrun_dir: &Path, libkrunfw_dir: &Path) {
    println!("cargo:rustc-link-search=native={}", libkrun_dir.display());
    println!("cargo:rustc-link-lib=dylib=krun");

    // Expose library directories to downstream crates (used by boxlite/build.rs)
    // Convention: {LIBNAME}_BOXLITE_DEP=<path> for auto-discovery
    println!("cargo:LIBKRUN_BOXLITE_DEP={}", libkrun_dir.display());
    println!("cargo:LIBKRUNFW_BOXLITE_DEP={}", libkrunfw_dir.display());
}

/// Fixes the install_name on macOS to use an absolute path.
/// This allows install_name_tool to modify the library path during wheel repair.
#[cfg(target_os = "macos")]
fn fix_install_name(lib_name: &str, lib_path: &Path) {
    let status = Command::new("install_name_tool")
        .args([
            "-id",
            &format!("@rpath/{}", lib_name),
            lib_path.to_str().unwrap(),
        ])
        .status()
        .expect("Failed to execute install_name_tool");

    if !status.success() {
        panic!("Failed to set install_name for {}", lib_name);
    }
}

#[cfg(target_os = "linux")]
fn fix_install_name(lib_name: &str, lib_path: &Path) {
    let lib_path_str = lib_path.to_str().expect("Invalid library path");

    println!("cargo:warning=Fixing {} in {}", lib_name, lib_path_str);

    let status = Command::new("patchelf")
        .args([
            "--set-soname",
            lib_name, // On Linux, SONAME is just the library name, not @rpath/name
            lib_path_str,
        ])
        .status()
        .expect("Failed to execute patchelf");

    if !status.success() {
        panic!("Failed to set install_name for {}", lib_name);
    }
}

/// Copies libraries from Homebrew to OUT_DIR and fixes install_name to use @rpath
#[cfg(target_os = "macos")]
fn copy_and_fix_macos_libs(src_dir: &Path, out_dir: &Path, lib_prefix: &str) -> Result<(), String> {
    use std::collections::HashMap;
    use std::fs;

    // Remove old directory if it exists (clean slate)
    if out_dir.exists() {
        fs::remove_dir_all(out_dir)
            .map_err(|e| format!("Failed to remove old directory: {}", e))?;
    }

    fs::create_dir_all(out_dir).map_err(|e| format!("Failed to create output directory: {}", e))?;

    // Track symlinks to recreate them after copying real files
    let mut symlinks_to_create: HashMap<String, String> = HashMap::new();

    // First pass: copy regular files and record symlinks
    for entry in
        fs::read_dir(src_dir).map_err(|e| format!("Failed to read source directory: {}", e))?
    {
        let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
        let path = entry.path();
        let filename = path.file_name().unwrap().to_string_lossy().to_string();

        if filename.starts_with(lib_prefix) && filename.contains(".dylib") {
            let dest = out_dir.join(&filename);

            // Check if it's a symlink
            let metadata = fs::symlink_metadata(&path)
                .map_err(|e| format!("Failed to get metadata: {}", e))?;

            if metadata.file_type().is_symlink() {
                // Record symlink for later creation
                let target =
                    fs::read_link(&path).map_err(|e| format!("Failed to read symlink: {}", e))?;
                let target_name = target
                    .file_name()
                    .ok_or("Symlink target has no filename")?
                    .to_string_lossy()
                    .to_string();
                symlinks_to_create.insert(filename.clone(), target_name);
                println!(
                    "cargo:warning=Recorded symlink: {} -> {}",
                    filename,
                    target.display()
                );
            } else {
                // Copy regular file
                fs::copy(&path, &dest).map_err(|e| format!("Failed to copy file: {}", e))?;
                println!("cargo:warning=Copied library: {}", filename);

                // Fix install_name to use @rpath (only for non-symlinks)
                fix_install_name(&filename, &dest);

                // Re-sign after modifying (install_name_tool invalidates signature)
                let sign_status = Command::new("codesign")
                    .arg("-s")
                    .arg("-")
                    .arg("--force")
                    .arg(&dest)
                    .status()
                    .map_err(|e| format!("Failed to run codesign: {}", e))?;

                if !sign_status.success() {
                    return Err(format!("codesign failed for {}", filename));
                }
                println!("cargo:warning=Re-signed {}", filename);
            }
        }
    }

    // Second pass: recreate symlinks in OUT_DIR
    for (link_name, target_name) in symlinks_to_create {
        let link_path = out_dir.join(&link_name);
        let target_path = PathBuf::from(&target_name);

        std::os::unix::fs::symlink(&target_path, &link_path).map_err(|e| {
            format!(
                "Failed to create symlink {} -> {}: {}",
                link_name, target_name, e
            )
        })?;
        println!(
            "cargo:warning=Created symlink: {} -> {}",
            link_name, target_name
        );
    }

    Ok(())
}

/// Extract SONAME from versioned library filename
/// e.g., libkrunfw.so.4.9.0 -> Some("libkrunfw.so.4")
///       libkrun.so.1.15.1 -> Some("libkrun.so.1")
#[allow(dead_code)]
fn extract_major_soname(filename: &str) -> Option<String> {
    // Find ".so." pattern
    if let Some(so_pos) = filename.find(".so.") {
        let base = &filename[..so_pos + 3]; // "libkrunfw.so"
        let versions = &filename[so_pos + 4..]; // "4.9.0"

        // Get first number (major version)
        if let Some(major) = versions.split('.').next() {
            return Some(format!("{}.{}", base, major)); // "libkrunfw.so.4"
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn fix_linux_libs(src_dir: &Path, lib_prefix: &str) -> Result<(), String> {
    use std::fs;

    // First pass: copy regular files and record symlinks
    for entry in
        fs::read_dir(src_dir).map_err(|e| format!("Failed to read source directory: {}", e))?
    {
        let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
        let path = entry.path();
        let filename = path.file_name().unwrap().to_string_lossy().to_string();

        if filename.starts_with(lib_prefix) && filename.contains(".so") {
            // Check if it's a symlink
            let metadata = fs::symlink_metadata(&path)
                .map_err(|e| format!("Failed to get metadata: {}", e))?;

            if metadata.file_type().is_symlink() {
                continue;
            } else {
                // For libkrunfw only: rename to major version
                if lib_prefix == "libkrunfw" {
                    if let Some(soname) = extract_major_soname(&filename) {
                        if soname != filename {
                            let new_path = src_dir.join(&soname);
                            fs::rename(&path, &new_path)
                                .map_err(|e| format!("Failed to rename file: {}", e))?;
                            println!("cargo:warning=Renamed {} to {}", filename, soname);

                            // Fix install_name on renamed file
                            fix_install_name(&soname, &new_path);
                            continue;
                        }
                    }
                }

                // Fix install_name (only for non-symlinks)
                fix_install_name(&filename, &path);
            }
        }
    }

    Ok(())
}

/// macOS: Copy Homebrew libraries to OUT_DIR and fix install names
#[cfg(target_os = "macos")]
fn build() {
    println!("cargo:warning=Configuring libkrun-sys for macOS (copying from Homebrew)");

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let libkrun_out = out_dir.join("libkrun").join("lib");
    let libkrunfw_out = out_dir.join("libkrunfw").join("lib");

    // Find libkrun via pkg-config
    let libkrun_info = pkg_config::probe_library("libkrun").unwrap_or_else(|e| {
        eprintln!("ERROR: Failed to find libkrun via pkg-config: {}", e);
        eprintln!();
        eprintln!("Install libkrun using Homebrew:");
        eprintln!("  brew tap slp/krun && brew install libkrun");
        std::process::exit(1);
    });

    println!("cargo:warning=Found libkrun via pkg-config");

    // Find libkrun library directory
    let libkrun_src = libkrun_info
        .link_paths
        .iter()
        .find(|path| has_library(path, "libkrun."))
        .unwrap_or_else(|| {
            panic!("libkrun library not found in pkg-config link paths");
        });

    // Find libkrunfw via Homebrew
    let output = Command::new("brew")
        .args(["--prefix", "libkrunfw"])
        .output()
        .expect("Failed to run 'brew --prefix libkrunfw'");

    if !output.status.success() {
        eprintln!("ERROR: Failed to find libkrunfw via Homebrew");
        eprintln!();
        eprintln!("Install libkrunfw using Homebrew:");
        eprintln!("  brew tap slp/krun && brew install libkrunfw");
        std::process::exit(1);
    }

    let prefix = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let libkrunfw_src = PathBuf::from(prefix).join("lib");

    // Verify libkrunfw library exists
    if !has_library(&libkrunfw_src, "libkrunfw.") {
        panic!("libkrunfw library not found at {}", libkrunfw_src.display());
    }

    println!(
        "cargo:warning=Found libkrunfw at {}",
        libkrunfw_src.display()
    );

    // Copy libraries to OUT_DIR and fix install names
    copy_and_fix_macos_libs(libkrun_src, &libkrun_out, "libkrun")
        .unwrap_or_else(|e| panic!("Failed to copy libkrun: {}", e));

    copy_and_fix_macos_libs(&libkrunfw_src, &libkrunfw_out, "libkrunfw")
        .unwrap_or_else(|e| panic!("Failed to copy libkrunfw: {}", e));

    // Configure linking to use our copied versions
    configure_linking(&libkrun_out, &libkrunfw_out);
}

/// Linux: Build libkrun and libkrunfw from source
#[cfg(target_os = "linux")]
fn build() {
    println!("cargo:warning=Building libkrun-sys for Linux (from source)");

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    // Verify vendored sources exist
    let libkrunfw_src = manifest_dir.join("vendor/libkrunfw");
    let libkrun_src = manifest_dir.join("vendor/libkrun");

    if !libkrunfw_src.exists() || !libkrun_src.exists() {
        eprintln!("ERROR: Vendored sources not found");
        eprintln!();
        eprintln!("Initialize git submodules:");
        eprintln!("  git submodule update --init --recursive");
        std::process::exit(1);
    }

    // Build libkrunfw first (libkrun depends on it)
    let libkrunfw_install = out_dir.join("libkrunfw");
    build_with_make(
        &libkrunfw_src,
        &libkrunfw_install,
        "libkrunfw",
        HashMap::new(),
    );

    // Build libkrun with PKG_CONFIG_PATH pointing to libkrunfw and NET=1 BLK=1 features
    let libkrun_install = out_dir.join("libkrun");
    println!("cargo:warning=Building libkrun with NET=1 BLK=1 features");

    let libkrun_env = HashMap::from([
        (
            "PKG_CONFIG_PATH".to_string(),
            format!("{}/lib64/pkgconfig", libkrunfw_install.display()),
        ),
        ("NET".to_string(), "1".to_string()),
        ("BLK".to_string(), "1".to_string()),
    ]);

    build_with_make(&libkrun_src, &libkrun_install, "libkrun", libkrun_env);

    // Configure linking
    let libkrun_lib_dir = libkrun_install.join("lib64");
    fix_linux_libs(&libkrun_lib_dir, "libkrun")
        .unwrap_or_else(|e| panic!("Failed to fix libkrun: {}", e));

    let libkrunfw_lib_dir = libkrunfw_install.join("lib64");
    fix_linux_libs(&libkrunfw_lib_dir, "libkrunfw")
        .unwrap_or_else(|e| panic!("Failed to fix libkrunfw: {}", e));

    configure_linking(&libkrun_lib_dir, &libkrunfw_lib_dir);
}
