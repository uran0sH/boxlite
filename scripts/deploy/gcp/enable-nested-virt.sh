#!/bin/bash
# Enable nested virtualization on an existing GCP instance
#
# This will:
#   1. Export instance configuration to YAML
#   2. Add advancedMachineFeatures.enableNestedVirtualization: true
#   3. Update instance from modified configuration (auto-restarts if needed)
#   4. Optionally configure KVM group permissions
#
# Usage:
#   ./enable-nested-virt.sh --name my-instance --zone us-central1-a

set -euo pipefail

GCP_SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$GCP_SCRIPT_DIR/../../common.sh"

# ============================================================================
# Configuration
# ============================================================================

INSTANCE_NAME=""
ZONE="us-central1-a"
PROJECT=""
SKIP_KVM_SETUP=false
SKIP_CONFIRM=false

CONFIG_FILE=""

# ============================================================================
# Functions
# ============================================================================

usage() {
    cat << EOF
Usage: $(basename "$0") [OPTIONS]

Enable nested virtualization on an existing GCP instance.

WARNING: This will restart the instance.

OPTIONS:
    --name NAME         Instance name (required)
    --zone ZONE         GCP zone (default: us-central1-a)
    --project PROJECT   GCP project ID (uses gcloud default if not specified)
    --skip-kvm-setup    Skip automatic KVM permissions setup
    --yes               Skip confirmation prompt
    --help              Show this help message

EXAMPLES:
    $(basename "$0") --name my-instance --zone us-central1-a
    $(basename "$0") --name my-instance --zone us-central1-a --yes

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
            --skip-kvm-setup)
                SKIP_KVM_SETUP=true
                shift
                ;;
            --yes|-y)
                SKIP_CONFIRM=true
                shift
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

step_confirm() {
    print_header "Enabling nested virtualization on existing instance"

    print_info "Instance: $INSTANCE_NAME"
    print_info "Zone: $ZONE"
    print_warning "This will restart the instance."

    if [ "$SKIP_CONFIRM" = false ]; then
        read -p "Continue? (y/N) " -n 1 -r
        echo
        if [[ ! $REPLY =~ ^[Yy]$ ]]; then
            print_info "Aborted"
            exit 0
        fi
    fi
}

step_export_config() {
    print_info "Exporting instance configuration..."

    CONFIG_FILE="/tmp/${INSTANCE_NAME}-config.yaml"

    local project_flag=""
    if [ -n "$PROJECT" ]; then
        project_flag="--project=$PROJECT"
    fi

    gcloud compute instances export "$INSTANCE_NAME" \
        --destination="$CONFIG_FILE" \
        --zone="$ZONE" \
        $project_flag
}

step_check_already_enabled() {
    if grep -qi "enableNestedVirtualization: true" "$CONFIG_FILE"; then
        print_warning "Nested virtualization is already enabled in the configuration"
        rm -f "$CONFIG_FILE"
        "$GCP_SCRIPT_DIR/check-nested-virt.sh" \
            --name "$INSTANCE_NAME" \
            --zone "$ZONE" \
            ${PROJECT:+--project "$PROJECT"}
        exit 0
    fi
}

step_modify_config() {
    # Remove unsupported guestOsFeatures that gcloud update-from-file doesn't recognize
    print_info "Cleaning unsupported features from config..."
    sed -i.bak '/type: SNP_SVSM_CAPABLE/d' "$CONFIG_FILE"

    # Add nested virtualization flag
    print_info "Adding nested virtualization configuration..."

    if grep -q "advancedMachineFeatures:" "$CONFIG_FILE"; then
        # Add to existing section
        sed -i.bak2 '/advancedMachineFeatures:/a\
  enableNestedVirtualization: true
' "$CONFIG_FILE"
    else
        # Create new section after machineType
        sed -i.bak2 '/^machineType:/a\
advancedMachineFeatures:\
  enableNestedVirtualization: true
' "$CONFIG_FILE"
    fi
}

step_update_instance() {
    print_info "Updating instance configuration (this will restart the instance)..."

    local project_flag=""
    if [ -n "$PROJECT" ]; then
        project_flag="--project=$PROJECT"
    fi

    if gcloud compute instances update-from-file "$INSTANCE_NAME" \
        --source="$CONFIG_FILE" \
        --zone="$ZONE" \
        --most-disruptive-allowed-action=RESTART \
        $project_flag; then
        print_success "Instance configuration updated successfully"
    else
        print_error "Failed to update instance configuration"
        print_info "Configuration file saved at: $CONFIG_FILE"
        exit 1
    fi
}

step_cleanup() {
    rm -f "$CONFIG_FILE" "${CONFIG_FILE}.bak" "${CONFIG_FILE}.bak2"
}

step_wait_for_instance() {
    print_info "Waiting for instance to be ready..."
    sleep 10
}

step_verify_nested_virt() {
    "$GCP_SCRIPT_DIR/check-nested-virt.sh" \
        --name "$INSTANCE_NAME" \
        --zone "$ZONE" \
        ${PROJECT:+--project "$PROJECT"}
}

step_setup_kvm() {
    if [ "$SKIP_KVM_SETUP" = true ]; then
        print_info "Skipping KVM setup (--skip-kvm-setup)"
        return 0
    fi

    echo ""
    print_info "Setting up KVM permissions..."
    "$GCP_SCRIPT_DIR/setup-kvm.sh" \
        --name "$INSTANCE_NAME" \
        --zone "$ZONE" \
        ${PROJECT:+--project "$PROJECT"}
}

main() {
    require_command gcloud "Install: https://cloud.google.com/sdk/docs/install"

    parse_args "$@"
    validate_args

    step_confirm
    step_export_config
    step_check_already_enabled
    step_modify_config
    step_update_instance
    step_cleanup
    step_wait_for_instance
    step_verify_nested_virt
    step_setup_kvm
}

# ============================================================================
# Entry point
# ============================================================================

main "$@"
