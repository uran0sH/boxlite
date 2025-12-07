#!/bin/bash
# Create a new GCP instance with nested virtualization enabled
#
# Usage:
#   ./create-instance.sh --name my-instance --zone us-central1-a
#   ./create-instance.sh --name my-instance --zone us-west1-a --machine-type n2-standard-8

set -euo pipefail

GCP_SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$GCP_SCRIPT_DIR/../../common.sh"

# ============================================================================
# Configuration
# ============================================================================

DEFAULT_ZONE="us-central1-a"
DEFAULT_MACHINE_TYPE="n2-standard-4"
DEFAULT_IMAGE_FAMILY="ubuntu-2204-lts"
DEFAULT_IMAGE_PROJECT="ubuntu-os-cloud"
DEFAULT_BOOT_DISK_SIZE="50"

INSTANCE_NAME=""
ZONE="$DEFAULT_ZONE"
MACHINE_TYPE="$DEFAULT_MACHINE_TYPE"
IMAGE_FAMILY="$DEFAULT_IMAGE_FAMILY"
IMAGE_PROJECT="$DEFAULT_IMAGE_PROJECT"
BOOT_DISK_SIZE="$DEFAULT_BOOT_DISK_SIZE"
PROJECT=""
SKIP_KVM_SETUP=false

# ============================================================================
# Functions
# ============================================================================

usage() {
    cat << EOF
Usage: $(basename "$0") [OPTIONS]

Create a new GCP instance with nested virtualization enabled.

OPTIONS:
    --name NAME             Instance name (required)
    --zone ZONE             GCP zone (default: $DEFAULT_ZONE)
    --machine-type TYPE     Machine type (default: $DEFAULT_MACHINE_TYPE)
    --image-family FAMILY   Source image family (default: $DEFAULT_IMAGE_FAMILY)
    --image-project PROJECT Source image project (default: $DEFAULT_IMAGE_PROJECT)
    --boot-disk-size SIZE   Boot disk size in GB (default: $DEFAULT_BOOT_DISK_SIZE)
    --project PROJECT       GCP project ID (uses gcloud default if not specified)
    --skip-kvm-setup        Skip automatic KVM permissions setup
    --help                  Show this help message

EXAMPLES:
    $(basename "$0") --name boxlite-vm --zone us-west1-a
    $(basename "$0") --name boxlite-vm --zone us-west1-a --machine-type n2-standard-8 --boot-disk-size 100

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
            --machine-type)
                MACHINE_TYPE="$2"
                shift 2
                ;;
            --image-family)
                IMAGE_FAMILY="$2"
                shift 2
                ;;
            --image-project)
                IMAGE_PROJECT="$2"
                shift 2
                ;;
            --boot-disk-size)
                BOOT_DISK_SIZE="$2"
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

step_create_instance() {
    print_header "Creating GCP instance with nested virtualization"

    print_info "Instance: $INSTANCE_NAME"
    print_info "Zone: $ZONE"
    print_info "Machine type: $MACHINE_TYPE"

    local cmd="gcloud compute instances create $INSTANCE_NAME \
        --zone=$ZONE \
        --machine-type=$MACHINE_TYPE \
        --image-family=$IMAGE_FAMILY \
        --image-project=$IMAGE_PROJECT \
        --boot-disk-size=${BOOT_DISK_SIZE}GB \
        --boot-disk-type=pd-ssd \
        --min-cpu-platform='Intel Haswell' \
        --enable-nested-virtualization"

    if [ -n "$PROJECT" ]; then
        cmd="$cmd --project=$PROJECT"
    fi

    print_info "Running: $cmd"

    if eval "$cmd"; then
        print_success "Instance '$INSTANCE_NAME' created successfully"
    else
        print_error "Failed to create instance"
        exit 1
    fi
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

    step_create_instance
    step_wait_for_instance
    step_verify_nested_virt
    step_setup_kvm
}

# ============================================================================
# Entry point
# ============================================================================

main "$@"
