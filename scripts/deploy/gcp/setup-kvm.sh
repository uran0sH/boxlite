#!/bin/bash
# Set up KVM permissions on a GCP instance
#
# This SSHs into the instance and:
#   1. Verifies /dev/kvm exists
#   2. Adds current user to kvm group
#   3. Shows verification instructions
#
# Usage:
#   ./setup-kvm.sh --name my-instance --zone us-central1-a

set -euo pipefail

GCP_SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$GCP_SCRIPT_DIR/../../common.sh"

# ============================================================================
# Configuration
# ============================================================================

INSTANCE_NAME=""
ZONE="us-central1-a"
PROJECT=""

# ============================================================================
# Functions
# ============================================================================

usage() {
    cat << EOF
Usage: $(basename "$0") [OPTIONS]

Set up KVM group permissions on a GCP instance.

OPTIONS:
    --name NAME         Instance name (required)
    --zone ZONE         GCP zone (default: us-central1-a)
    --project PROJECT   GCP project ID (uses gcloud default if not specified)
    --help              Show this help message

EXAMPLES:
    $(basename "$0") --name my-instance --zone us-central1-a

EOF
    exit 0
}

parse_args() {
    while [[ $# -gt 0 ]]; do
        case $1 in
            --name)
                INSTANCE_NAME="$2"
                shift 2
                ;;
            --zone)
                ZONE="$2"
                shift 2
                ;;
            --project)
                PROJECT="$2"
                shift 2
                ;;
            --help|-h)
                usage
                ;;
            *)
                print_error "Unknown option: $1"
                usage
                ;;
        esac
    done
}

validate_args() {
    if [ -z "$INSTANCE_NAME" ]; then
        print_error "Instance name required (use --name)"
        exit 1
    fi
}

step_configure_kvm() {
    print_header "Setting up KVM permissions"
    print_info "Instance: $INSTANCE_NAME"
    print_info "Zone: $ZONE"

    local ssh_cmd="gcloud compute ssh $INSTANCE_NAME --zone=$ZONE"
    if [ -n "$PROJECT" ]; then
        ssh_cmd="$ssh_cmd --project=$PROJECT"
    fi

    print_info "SSHing into instance to configure kvm group..."

    local setup_script='
set -e
echo "==> Checking /dev/kvm..."
if [ ! -e /dev/kvm ]; then
    echo "ERROR: /dev/kvm not found. Nested virtualization may not be enabled."
    exit 1
fi

echo "==> Checking kvm group..."
if ! getent group kvm > /dev/null; then
    echo "ERROR: kvm group does not exist"
    exit 1
fi

echo "==> Adding $USER to kvm group..."
sudo usermod -aG kvm $USER

echo "==> Current permissions:"
ls -l /dev/kvm
echo ""
echo "==> User groups (after update):"
groups $USER

echo ""
echo "Setup complete!"
echo ""
echo "IMPORTANT: You need to log out and log back in for group changes to take effect."
echo "After re-login, verify with: groups | grep kvm"
'

    if $ssh_cmd --command="$setup_script"; then
        print_success "KVM permissions configured successfully"
        print_warning "Remember to log out and back in for group changes to take effect"
    else
        print_error "Failed to configure KVM permissions"
        exit 1
    fi
}

main() {
    require_command gcloud "Install: https://cloud.google.com/sdk/docs/install"

    parse_args "$@"
    validate_args

    step_configure_kvm
}

# ============================================================================
# Entry point
# ============================================================================

main "$@"
