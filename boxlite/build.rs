use regex::Regex;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

/// Copies all dynamic library files from source directory to destination.
/// Only copies files with library extensions (.dylib, .so, .so.*, .dll).
/// Preserves symlinks to avoid duplicating the same library multiple times.
fn copy_libs(source: &Path, dest: &Path) -> Result<(), String> {
    if !source.exists() {
        return Err(format!(
            "Source directory does not exist: {}",
            source.display()
        ));
    }

    fs::create_dir_all(dest).map_err(|e| {
        format!(
            "Failed to create destination directory {}: {}",
            dest.display(),
            e
        )
    })?;

    for entry in fs::read_dir(source).map_err(|e| {
        format!(
            "Failed to read source directory {}: {}",
            source.display(),
            e
        )
    })? {
        let entry = entry.map_err(|e| format!("Failed to read directory entry: {}", e))?;
        let source_path = entry.path();

        let file_name = source_path.file_name().ok_or("Failed to get filename")?;

        // Only process library files
        if !is_library_file(&source_path) {
            continue;
        }

        let dest_path = dest.join(file_name);

        // Check if source is a symlink
        let metadata = fs::symlink_metadata(&source_path).map_err(|e| {
            format!(
                "Failed to read metadata for {}: {}",
                source_path.display(),
                e
            )
        })?;

        if metadata.file_type().is_symlink() {
            // Skip symlinks - runtime linker uses the full versioned name embedded in the binary
            // (e.g., @rpath/libkrun.1.15.1.dylib, not @rpath/libkrun.dylib)
            // Symlinks are only needed during build-time linking
            continue;
        }

        if metadata.is_file() {
            // Regular file - remove existing file first (maybe read-only)
            if dest_path.exists() {
                fs::remove_file(&dest_path).map_err(|e| {
                    format!(
                        "Failed to remove existing file {}: {}",
                        dest_path.display(),
                        e
                    )
                })?;
            }

            // Copy the file
            fs::copy(&source_path, &dest_path).map_err(|e| {
                format!(
                    "Failed to copy {} -> {}: {}",
                    source_path.display(),
                    dest_path.display(),
                    e
                )
            })?;

            println!(
                "cargo:warning=Bundled library: {}",
                file_name.to_string_lossy()
            );
        }
    }

    Ok(())
}

/// Checks if a file is a dynamic library based on its extension.
fn is_library_file(path: &Path) -> bool {
    let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

    // macOS: .dylib
    if filename.ends_with(".dylib") {
        return true;
    }

    // Linux: .so or .so.VERSION
    if filename.contains(".so") {
        return true;
    }

    // Windows: .dll
    if filename.ends_with(".dll") {
        return true;
    }

    false
}

/// Auto-discovers and bundles all FFI library dependencies from -sys crates.
///
/// Convention: Each -sys crate emits `cargo:{LIBNAME}_BOXLITE_DEP=<path>`
/// which becomes `DEP_{LINKS}_{LIBNAME}_BOXLITE_DEP` env var.
///
/// Returns a list of bundled library names.
fn bundle_boxlite_deps(runtime_dir: &Path) -> Vec<String> {
    // Pattern: DEP_{LINKS}_{LIBNAME}_BOXLITE_DEP
    // Example: DEP_KRUN_LIBKRUN_BOXLITE_DEP -> libkrun
    let re = Regex::new(r"^DEP_[A-Z]+_([A-Z]+)_BOXLITE_DEP$").unwrap();

    let mut collected_libs = Vec::new();

    for (key, lib_dir) in env::vars() {
        if let Some(caps) = re.captures(&key) {
            let lib_name = caps[1].to_lowercase();

            println!(
                "cargo:warning=Found dependency: {} on {}",
                lib_name, lib_dir
            );

            match copy_libs(Path::new(&lib_dir), runtime_dir) {
                Ok(()) => {
                    collected_libs.push(lib_name);
                }
                Err(e) => {
                    panic!("Failed to copy {}: {}", lib_name, e);
                }
            }
        }
    }

    collected_libs
}

/// Collects all FFI library dependencies into a single runtime directory.
/// This directory can be used by downstream crates (e.g., Python SDK) to
/// bundle all required libraries together.
fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let runtime_dir = out_dir.join("runtime");

    // Force rerun if runtime directory doesn't exist or is empty
    if !runtime_dir.exists()
        || fs::read_dir(&runtime_dir)
            .map(|mut d| d.next().is_none())
            .unwrap_or(true)
    {
        println!("cargo:rerun-if-changed=FORCE_REBUILD");
    }

    // Create the runtime directory
    fs::create_dir_all(&runtime_dir)
        .unwrap_or_else(|e| panic!("Failed to create runtime directory: {}", e));

    // Auto-discover and bundle all FFI library dependencies from -sys crates
    let collected_libs = bundle_boxlite_deps(&runtime_dir);
    // Expose the runtime directory to downstream crates (e.g., Python SDK)
    println!("cargo:runtime_dir={}", runtime_dir.display());
    if !collected_libs.is_empty() {
        println!(
            "cargo:warning=Bundled runtime libraries: {}",
            collected_libs.join(", ")
        );
    }

    // Set rpath for boxlite-shim
    #[cfg(target_os = "macos")]
    println!("cargo:rustc-link-arg=-Wl,-rpath,@loader_path");
    #[cfg(target_os = "linux")]
    println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN");
}
