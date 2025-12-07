#!/bin/bash
# Setup script for BoxLite development on macOS
#
# This script installs all required dependencies for building BoxLite on macOS.
# Run this once when setting up a new development environment.

set -e

# Source common utilities
SETUP_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT_DIR="$(cd "$SETUP_DIR/.." && pwd)"
source "$SCRIPT_DIR/common.sh"
source "$SETUP_DIR/setup-common.sh"

# Check if running on macOS
check_platform() {
    if [[ "$(uname)" != "Darwin" ]]; then
        print_error "This script is for macOS only"
        echo "   For Ubuntu/Debian, use: bash scripts/setup/setup-ubuntu.sh"
        echo "   For manylinux/RHEL/CentOS, use: bash scripts/setup/setup-manylinux.sh"
        exit 1
    fi
}

# Check if a Homebrew package is installed
brew_installed() {
    brew list "$1" &>/dev/null
}

# Check if a Homebrew tap is tapped
brew_tapped() {
    brew tap | grep -q "^$1$"
}

# Check and install Homebrew
setup_homebrew() {
    print_step "Checking for Homebrew... "
    if command_exists brew; then
        print_success "Found"
    else
        print_error "Not found"
        echo ""
        print_section "Installing Homebrew..."
        /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"

        # Add Homebrew to PATH for Apple Silicon Macs
        if [[ -f "/opt/homebrew/bin/brew" ]]; then
            eval "$(/opt/homebrew/bin/brew shellenv)"
        fi

        print_success "Homebrew installed"
    fi
}

# Update Homebrew
update_homebrew() {
    print_section "🔄 Updating Homebrew..."
    brew update
    echo ""
}

# Setup Rust
setup_rust() {
    if ! check_rust; then
        install_rust
        export RUST_JUST_INSTALLED=true
    fi
    echo ""
}

# Setup Rust target
setup_rust_target() {
    detect_guest_target
    check_rust_target "$GUEST_TARGET"
    echo ""
}

# Install musl-cross
install_musl_cross() {
    print_step "Checking for musl-cross... "
    if brew_installed "musl-cross"; then
        print_success "Already installed"
    else
        echo -e "${YELLOW}Installing...${NC}"
        brew install FiloSottile/musl-cross/musl-cross
        print_success "musl-cross installed"
    fi
    echo ""
}

# Tap slp/krun repository
tap_krun_repo() {
    print_step "Checking for slp/krun tap... "
    if brew_tapped "slp/krun"; then
        print_success "Already tapped"
    else
        echo -e "${YELLOW}Tapping...${NC}"
        brew tap slp/krun
        print_success "slp/krun tapped"
    fi
    echo ""
}

# Install libkrun
install_libkrun() {
    print_step "Checking for libkrun... "
    if brew_installed "libkrun"; then
        print_success "Already installed"
    else
        echo -e "${YELLOW}Installing...${NC}"
        brew install libkrun
        print_success "libkrun installed"
    fi
    echo ""
}

# Install libkrunfw
install_libkrunfw() {
    print_step "Checking for libkrunfw... "
    if brew_installed "libkrunfw"; then
        print_success "Already installed"
    else
        echo -e "${YELLOW}Installing...${NC}"
        brew install libkrunfw
        print_success "libkrunfw installed"
    fi
    echo ""
}

# Install dylibbundler
install_dylibbundler() {
    print_step "Checking for dylibbundler... "
    if brew_installed "dylibbundler"; then
        print_success "Already installed"
    else
        echo -e "${YELLOW}Installing...${NC}"
        brew install dylibbundler
        print_success "dylibbundler installed"
    fi
    echo ""
}

# Install protobuf (for boxlite-shared gRPC/protobuf compilation)
install_protobuf() {
    print_step "Checking for protobuf... "
    if brew_installed "protobuf"; then
        print_success "Already installed"
    else
        echo -e "${YELLOW}Installing...${NC}"
        brew install protobuf
        print_success "protobuf installed"
    fi
    echo ""
}

# Setup Python
setup_python() {
    if ! check_python; then
        echo -e "${YELLOW}Installing...${NC}"
        brew install python@3.11
        print_success "Python installed"
    fi
    echo ""
}

# Setup Go
setup_go() {
    if ! check_go; then
        echo -e "${YELLOW}Installing...${NC}"
        brew install go
        print_success "Go installed"
    fi
    echo ""
}

# Verify library installation
verify_libraries() {
    print_section "🔍 Verifying library installation..."
    echo ""

    print_step "Checking pkg-config for libkrun... "
    if pkg-config --exists libkrun; then
        local libkrun_version=$(pkg-config --modversion libkrun)
        print_success "Found (version $libkrun_version)"
    else
        print_error "Not found via pkg-config"
        print_warning "This might cause build issues"
    fi

    print_step "Checking pkg-config for libkrunfw... "
    if pkg-config --exists libkrunfw; then
        local libkrunfw_version=$(pkg-config --modversion libkrunfw)
        print_success "Found (version $libkrunfw_version)"
    else
        # libkrunfw might not have a .pc file, check via Homebrew instead
        if brew_installed "libkrunfw"; then
            echo -e "${YELLOW}✓ Installed via Homebrew (no .pc file)${NC}"
        else
            print_error "Not found"
        fi
    fi
    echo ""
}

# Main installation flow
main() {
    print_header "BoxLite Development Setup for macOS"

    check_platform

    print_section "📋 Checking prerequisites..."
    echo ""

    setup_homebrew
    echo ""

    update_homebrew

    init_submodules

    setup_rust

    setup_rust_target

    install_musl_cross

    tap_krun_repo

    install_libkrun

    install_libkrunfw

    install_dylibbundler

    install_protobuf

    setup_python

    setup_go

    verify_libraries

    print_header "Setup Complete"
}

main "$@"
