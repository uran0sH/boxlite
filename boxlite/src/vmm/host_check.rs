//! Virtualization support detection for VMM engines.
//!
//! This module validates that the host system supports the required
//! virtualization technology (KVM on Linux, Hypervisor.framework on macOS).
//!
//! These checks follow "Validate Early" - fail fast before expensive
//! initialization work like filesystem setup and database creation.

use boxlite_shared::{BoxliteError, BoxliteResult};

/// Result of successful virtualization support detection.
///
/// Contains human-readable confirmation that virtualization is available.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtualizationSupport {
    /// Human-readable reason for support status
    pub reason: String,
}

/// Check if the host system supports the required virtualization technology.
///
/// This validates platform prerequisites:
/// - **Linux**: KVM (`/dev/kvm` accessibility)
/// - **macOS**: Hypervisor.framework (via `sysctl kern.hv_support`)
///
/// Follows "Validate Early" principle - fail fast with clear diagnostics
/// instead of obscure errors from libkrun later.
///
/// # Errors
///
/// Returns `BoxliteError::Unsupported` with diagnostic information if:
/// - Linux: `/dev/kvm` doesn't exist or isn't accessible
/// - macOS: Wrong architecture (only ARM64 supported) or Hypervisor.framework unavailable
///
/// # Examples
///
/// ```no_run
/// use boxlite::vmm::host_check::check_virtualization_support;
///
/// match check_virtualization_support() {
///     Ok(support) => {
///         println!("Virtualization supported: {}", support.reason);
///     }
///     Err(e) => {
///         eprintln!("Virtualization not supported: {}", e);
///     }
/// }
/// ```
pub fn check_virtualization_support() -> BoxliteResult<VirtualizationSupport> {
    #[cfg(target_os = "linux")]
    {
        check_linux_kvm()
    }

    #[cfg(target_os = "macos")]
    {
        check_macos_hypervisor()
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Err(BoxliteError::Unsupported(
            "Boxlite only supports Linux and macOS".into(),
        ))
    }
}

/// Linux KVM support detection.
///
/// Verifies that `/dev/kvm` exists and is accessible by the current user.
///
/// # Errors
///
/// Returns `BoxliteError::Unsupported` if KVM is not available or accessible.
#[cfg(target_os = "linux")]
fn check_linux_kvm() -> BoxliteResult<VirtualizationSupport> {
    use std::path::Path;

    const KVM_DEVICE: &str = "/dev/kvm";
    let kvm_path = Path::new(KVM_DEVICE);

    // Check if /dev/kvm exists
    if !kvm_path.exists() {
        let mut suggestions = format!(
            "{} does not exist\n\n\
                   Suggestions:\n\
                   - Enable KVM in your BIOS/UEFI settings (VT-x for Intel, AMD-V for AMD)\n\
                   - Ensure your kernel is compiled with KVM support\n\
                   - Check if kvm module is loaded: lsmod | grep kvm\n\
                   - Try: sudo modprobe kvm_intel  # Intel\n\
                          sudo modprobe kvm_amd    # AMD",
            KVM_DEVICE
        );

        // Detect WSL2 and add specific guidance
        if std::path::Path::new("/proc/sys/fs/binfmt_misc/WSLInterop").exists() {
            suggestions.push_str(
                      "\n\nWSL2 detected:\n\
                       - Requires Windows 11 or Windows 10 build 21390+\n\
                       - Enable nested virtualization: add 'nestedVirtualization=true' to .wslconfig\n\
                       - Restart WSL: wsl --shutdown"
                  );
        }

        return Err(BoxliteError::Unsupported(suggestions));
    }

    // Check if /dev/kvm is accessible
    match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(kvm_path)
    {
        Ok(_) => Ok(VirtualizationSupport {
            reason: "KVM is available and accessible".to_string(),
        }),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            Err(BoxliteError::Unsupported(format!(
                "{} exists but access denied (permissions)\n\n\
                 Suggestions:\n\
                 - Add your user to the kvm group: sudo usermod -aG kvm $USER\n\
                 - Log out and log back in for group changes to take effect\n\
                 - Verify group membership: groups\n\
                 - Check permissions: ls -l {}",
                KVM_DEVICE, KVM_DEVICE
            )))
        }
        Err(e) => Err(BoxliteError::Unsupported(format!(
            "{} exists but couldn't be accessed: {}\n\n\
             Suggestions:\n\
             - Check if another VM process is locking the device\n\
             - Review system logs: dmesg | tail -50\n\
             - Ensure KVM modules are loaded correctly",
            KVM_DEVICE, e
        ))),
    }
}

/// macOS Hypervisor.framework support detection.
///
/// Only Apple Silicon (ARM64) is supported. Intel Macs are not supported.
///
/// Uses `sysctl kern.hv_support` to reliably detect Hypervisor.framework availability.
///
/// # Errors
///
/// Returns `BoxliteError::Unsupported` if architecture is not ARM64 or
/// Hypervisor.framework is not available.
#[cfg(target_os = "macos")]
fn check_macos_hypervisor() -> BoxliteResult<VirtualizationSupport> {
    #[cfg(target_arch = "aarch64")]
    {
        use std::process::Command;

        // Query Hypervisor.framework availability via sysctl
        let output = Command::new("sysctl")
            .arg("kern.hv_support")
            .output()
            .map_err(|e| {
                BoxliteError::Unsupported(format!(
                    "Failed to check Hypervisor.framework support: {}\n\n\
                     Suggestions:\n\
                     - Verify macOS version and system integrity\n\
                     - Check manually: sysctl kern.hv_support",
                    e
                ))
            })?;

        if !output.status.success() {
            return Err(BoxliteError::Unsupported(
                "sysctl command failed\n\n\
                 Suggestions:\n\
                 - Verify macOS version and system integrity\n\
                 - Check manually: sysctl kern.hv_support"
                    .to_string(),
            ));
        }

        // Parse output: "kern.hv_support: 1" or "kern.hv_support: 0"
        let stdout = String::from_utf8_lossy(&output.stdout);
        let value = stdout.split(':').nth(1).map(|s| s.trim()).unwrap_or("0");
        const HYPERVISOR_SUPPORTED: &str = "1";

        if value == HYPERVISOR_SUPPORTED {
            Ok(VirtualizationSupport {
                reason: "Hypervisor.framework is available (Apple Silicon)".to_string(),
            })
        } else {
            Err(BoxliteError::Unsupported(
                "Hypervisor.framework is not available\n\n\
                 Suggestions:\n\
                 - Verify you're on macOS 10.10 or later\n\
                 - Check system requirements: sysctl kern.hv_support\n\
                 - Ensure virtualization is enabled in your system settings"
                    .to_string(),
            ))
        }
    }

    #[cfg(not(target_arch = "aarch64"))]
    {
        Err(BoxliteError::Unsupported(format!(
            "Unsupported architecture: {}\n\n\
             Suggestions:\n\
             - Boxlite on macOS requires Apple Silicon (ARM64)\n\
             - Intel Macs are not supported",
            std::env::consts::ARCH
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(target_os = "linux")]
    fn test_linux_kvm_check_runs() {
        // This test verifies the check runs without panicking
        // Result depends on whether /dev/kvm exists in test environment
        match check_linux_kvm() {
            Ok(support) => {
                assert!(
                    support.reason.contains("KVM") || support.reason.contains("accessible"),
                    "Reason should mention KVM or accessible"
                );
            }
            Err(e) => {
                // Expected in environments without KVM (containers, CI)
                assert!(e.to_string().contains("KVM") || e.to_string().contains("/dev/kvm"));
            }
        }
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn test_macos_hypervisor_check_runs() {
        // This test verifies the check runs without panicking
        match check_macos_hypervisor() {
            Ok(support) => {
                assert!(
                    support.reason.contains("Hypervisor")
                        || support.reason.contains("Apple Silicon"),
                    "Reason should mention Hypervisor or Apple Silicon"
                );
            }
            Err(e) => {
                // Expected on Intel Macs or without Hypervisor.framework
                assert!(
                    e.to_string().contains("architecture") || e.to_string().contains("Hypervisor")
                );
            }
        }
    }
}
