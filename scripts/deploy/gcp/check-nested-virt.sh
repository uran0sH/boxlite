#!/bin/bash
# Check if nested virtualization is enabled on a GCP instance
#
# Usage:
#   ./check-nested-virt.sh --name my-instance --zone us-central1-a

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

Check if nested virtualization is enabled on a GCP instance.

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

step_query_instance() {
    print_info "Checking nested virtualization status for $INSTANCE_NAME..."

    local project_flag=""
    if [ -n "$PROJECT" ]; then
        project_flag="--project=$PROJECT"
    fi

    local retries=0
    local max_retries=6
    NESTED_VIRT=""

    while [ $retries -lt $max_retries ]; do
        NESTED_VIRT=$(gcloud compute instances describe "$INSTANCE_NAME" \
            --zone="$ZONE" \
            $project_flag \
            --format='get(advancedMachineFeatures.enableNestedVirtualization)' 2>/dev/null || echo "")

        if [ -n "$NESTED_VIRT" ] || [ $retries -ge 2 ]; then
            break
        fi

        retries=$((retries + 1))
        print_info "Instance not ready yet, waiting... (attempt $retries/$max_retries)"
        sleep 10
    done
}

step_report_status() {
    if [ "$NESTED_VIRT" = "True" ]; then
        print_success "Nested virtualization is ENABLED on $INSTANCE_NAME"
        echo ""
        print_info "To verify inside the instance, SSH in and run:"
        echo ""
        echo "    # Check CPU supports virtualization"
        echo "    grep -cw vmx\\|svm /proc/cpuinfo"
        echo ""
        echo "    # Verify /dev/kvm exists"
        echo "    ls -l /dev/kvm"
        echo ""
        echo "    # Check kvm group membership"
        echo "    groups | grep kvm"
        echo ""
        return 0
    elif [ "$NESTED_VIRT" = "False" ] || [ -z "$NESTED_VIRT" ]; then
        print_warning "Nested virtualization is NOT enabled on $INSTANCE_NAME (value: ${NESTED_VIRT:-none})"
        return 1
    else
        print_error "Unable to determine nested virtualization status"
        return 1
    fi
}

main() {
    require_command gcloud "Install: https://cloud.google.com/sdk/docs/install"

    parse_args "$@"
    validate_args

    step_query_instance
    step_report_status
}

# ============================================================================
# Entry point
# ============================================================================

main "$@"
