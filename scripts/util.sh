#!/bin/bash
# Build utility functions for BoxLite scripts
#
# This script provides utility functions for architecture detection,
# platform detection, and other common build tasks.
#
# Usage:
#   source scripts/util.sh
#   echo "Building for $GUEST_TARGET"
#
# Or as a command:
#   GUEST_TARGET=$(scripts/util.sh --target)

set -e

# Detect host architecture
detect_host_arch() {
    uname -m
}

# Map architecture to Linux musl target triple
map_arch_to_target() {
    local arch="$1"

    case "$arch" in
        arm64|aarch64)
            echo "aarch64-unknown-linux-gnu"
            ;;
        x86_64|amd64)
            echo "x86_64-unknown-linux-gnu"
            ;;
        *)
            echo "ERROR: Unsupported architecture: $arch" >&2
            echo "Supported: arm64, aarch64, x86_64, amd64" >&2
            return 1
            ;;
    esac
}

# Normalize architecture name
normalize_arch() {
    local arch="$1"

    case "$arch" in
        arm64|aarch64)
            echo "aarch64"
            ;;
        x86_64|amd64)
            echo "x86_64"
            ;;
        *)
            echo "ERROR: Unsupported architecture: $arch" >&2
            return 1
            ;;
    esac
}

# Initialize guest target and arch variables
init_guest_vars() {
    local arch=$(detect_host_arch)
    GUEST_TARGET=$(map_arch_to_target "$arch")
    GUEST_ARCH=$(normalize_arch "$arch")

    # Export for use in other scripts
    export GUEST_TARGET
    export GUEST_ARCH
}

# Print help message
print_help() {
    cat <<EOF
Usage: util.sh [OPTION]

Build utility functions for BoxLite scripts.

Options:
  --target    Print the full Rust target triple (e.g., aarch64-unknown-linux-musl)
  --arch      Print just the architecture (e.g., aarch64)
  --help      Show this help message

When sourced, sets environment variables:
  GUEST_TARGET    Full Rust target triple
  GUEST_ARCH      Architecture name

Examples:
  # Source in a script:
  source scripts/util.sh
  cargo build --target \$GUEST_TARGET

  # Use as a command:
  GUEST_TARGET=\$(scripts/util.sh --target)
  echo "Building for \$GUEST_TARGET"

EOF
}

# Main execution
main() {
    # Initialize variables
    init_guest_vars

    # If run as a command (not sourced), print the requested value
    if [ "${BASH_SOURCE[0]}" = "${0}" ]; then
        case "${1:-}" in
            --target)
                echo "$GUEST_TARGET"
                ;;
            --arch)
                echo "$GUEST_ARCH"
                ;;
            --help|-h)
                print_help
                ;;
            "")
                # Default: print both
                echo "GUEST_TARGET=$GUEST_TARGET"
                echo "GUEST_ARCH=$GUEST_ARCH"
                ;;
            *)
                echo "ERROR: Unknown option: $1" >&2
                echo "Run with --help for usage information" >&2
                exit 1
                ;;
        esac
    fi
}

main "$@"
