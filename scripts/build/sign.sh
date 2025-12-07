#!/bin/bash
# Sign boxlite-shim with required entitlements for macOS

set -e

# Load common utilities
SCRIPT_BUILD_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT_DIR="$(cd "$SCRIPT_BUILD_DIR/.." && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
source "$SCRIPT_DIR/common.sh"

# Capture original working directory before any cd commands
ORIG_DIR="$(pwd)"

BINARY="${1}"

if [ ! -f "$BINARY" ]; then
    print_error "Binary not found at $BINARY"
    exit 1
fi

print_header "🔏 Signing binary..."

print_section "Signing $BINARY with hypervisor entitlement..."

codesign -s - --force --entitlements /dev/stdin "$BINARY" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>com.apple.security.hypervisor</key>
	<true/>
	<key>com.apple.security.cs.disable-library-validation</key>
	<true/>
</dict>
</plist>
EOF

print_success "Binary signed successfully"
codesign -d -v "$BINARY" 2>&1 | grep -E "(Identifier|Signature)"
