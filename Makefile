.PHONY: help clean setup package dev\:python dist dist\:python test fmt fmt-check guest runtime runtime-debug

# Ensure cargo is in PATH (source ~/.cargo/env if it exists and cargo is not found)
SHELL := /bin/bash
export PATH := $(HOME)/.cargo/bin:$(PATH)

PROJECT_ROOT := $(shell pwd)
SCRIPT_DIR := $(PROJECT_ROOT)/scripts


help:
	@echo "BoxLite Build Commands:"
	@echo ""
	@echo "  Setup:"
	@echo "    make setup          - Install all dependencies (auto-detects: macOS/Ubuntu/manylinux/musllinux)"
	@echo ""
	@echo "  Cleanup:"
	@echo "    make clean          - Clean build artifacts (keep .venv)"
	@echo "    make clean:all      - Clean everything including .venv"
	@echo "    make clean:dist     - Clean SDK distribution artifacts"
	@echo ""
	@echo "  Code Quality:"
	@echo "    make fmt            - Format all Rust code"
	@echo "    make fmt-check      - Check Rust formatting without modifying files"
	@echo ""
	@echo "  Build:"
	@echo "    make guest          - Build the guest binary (cross-compile for VM)"
	@echo ""
	@echo "  Local Development:"
	@echo "    make dev:python     - Build and install Python SDK locally (debug mode)"
	@echo "    make test           - Test the built wheel"
	@echo ""
	@echo "  Python Distribution:"
	@echo "    make dist:python    - Build portable wheel with cibuildwheel (auto-detects platform)"
	@echo ""
	@echo "  Library Distribution:"
	@echo "    make package        - Package libboxlite for current platform"
	@echo ""
	@echo "Platform: $$(uname) ($$(uname -m))"
	@echo ""

clean:
	@$(SCRIPT_DIR)/clean.sh --mode runtime

clean\:%:
	@$(SCRIPT_DIR)/clean.sh --mode $(subst clean:,,$@)

setup:
	@if [ "$$(uname)" = "Darwin" ]; then \
		bash $(SCRIPT_DIR)/setup/setup-macos.sh; \
	elif [ "$$(uname)" = "Linux" ]; then \
		if [ -f /etc/os-release ] && grep -q "manylinux" /etc/os-release 2>/dev/null; then \
			bash $(SCRIPT_DIR)/setup/setup-manylinux.sh; \
		elif [ -f /etc/os-release ] && grep -q "musllinux" /etc/os-release 2>/dev/null; then \
			bash $(SCRIPT_DIR)/setup/setup-musllinux.sh; \
		elif command -v apt-get >/dev/null 2>&1; then \
			bash $(SCRIPT_DIR)/setup/setup-ubuntu.sh; \
		elif command -v apk >/dev/null 2>&1; then \
			bash $(SCRIPT_DIR)/setup/setup-musllinux.sh; \
		elif command -v yum >/dev/null 2>&1; then \
			bash $(SCRIPT_DIR)/setup/setup-manylinux.sh; \
		else \
			echo "❌ Unsupported Linux distribution"; \
			echo "   Supported: Ubuntu/Debian (apt-get), RHEL/CentOS/manylinux (yum), or Alpine/musllinux (apk)"; \
			exit 1; \
		fi; \
	else \
		echo "❌ Unsupported platform: $$(uname)"; \
		exit 1; \
	fi

guest:
	@bash $(SCRIPT_DIR)/build/build-guest.sh

runtime:
	@$(SCRIPT_DIR)/clean.sh --mode runtime $(if $(filter 1,$(KEEP_GUEST_BIN)),--keep-guest-bin)
	@bash $(SCRIPT_DIR)/build/prepare-runtime.sh

runtime-debug:
	@$(SCRIPT_DIR)/clean.sh --mode runtime
	@bash $(SCRIPT_DIR)/build/prepare-runtime.sh --profile debug

dist\:python:
	@if [ ! -d .venv ]; then \
		echo "📦 Creating virtual environment..."; \
		python3 -m venv .venv; \
	fi

	echo "📦 Installing cibuildwheel..."; \
	. .venv/bin/activate && pip install -q cibuildwheel; \

	@if [ "$$(uname)" = "Darwin" ]; then \
  		source .venv/bin/activate; \
  		cibuildwheel --only cp314-macosx_arm64 sdks/python; \
	    cibuildwheel --platform linux sdks/python; \
	    # CIBW_CONTAINER_ENGINE="docker; create_args: --cpus=10 --memory=16g" CIBW_DEBUG_KEEP_CONTAINER=1 cibuildwheel --platform linux sdks/python; \
	elif [ "$$(uname)" = "Linux" ]; then \
  		source .venv/bin/activate; \
	    bash $(SCRIPT_DIR)/build/build-guest.sh; \
	    cibuildwheel --platform linux sdks/python; \
	    # CIBW_CONTAINER_ENGINE="docker; create_args: --cpus=10 --memory=16g" CIBW_DEBUG_KEEP_CONTAINER=1 cibuildwheel --platform linux sdks/python; \
	else \
		echo "❌ Unsupported platform: $$(uname)"; \
		exit 1; \
	fi

dist\:c: runtime
	@if [ "$$(uname)" = "Darwin" ]; then \
		bash $(SCRIPT_DIR)/package/package-macos.sh $(ARGS); \
	elif [ "$$(uname)" = "Linux" ]; then \
		bash $(SCRIPT_DIR)/package/package-linux.sh $(ARGS); \
	else \
		echo "❌ Unsupported platform: $$(uname)"; \
		exit 1; \
	fi

# Build wheel locally with maturin + platform-specific repair tool
dev\:python: runtime-debug
	@echo "📦 Building wheel locally with maturin..."
	@if [ ! -d .venv ]; then \
		echo "📦 Creating virtual environment..."; \
		python3 -m venv .venv; \
	fi

	echo "📦 Installing maturin..."; \
	. .venv/bin/activate && pip install -q maturin; \

	@echo "📦 Copying runtime to Python module..."
	@rm -rf $(CURDIR)/sdks/python/boxlite/runtime
	@cp -a $(CURDIR)/target/boxlite-runtime $(CURDIR)/sdks/python/boxlite/runtime

	@echo "🔨 Building wheel with maturin..."
	@. .venv/bin/activate && cd sdks/python && maturin develop

# Test the built wheel
test:
	@echo "🧪 Testing wheel..."
	@if [ ! -f wheelhouse/*.whl ]; then \
		echo "❌ No wheel found. Run 'make dist' first."; \
		exit 1; \
	fi
	@if [ ! -d .venv ]; then \
		echo "📦 Creating virtual environment..."; \
		python3 -m venv .venv; \
	fi
	@. .venv/bin/activate && pip install -q wheelhouse/*.whl --force-reinstall
	@. .venv/bin/activate && python -c "import boxlite; print(f'✓ BoxLite {boxlite.__version__} imported successfully')"
	@echo "🧪 Running examples..."
	@. .venv/bin/activate && python examples/python/execute.py

# Format all Rust code
fmt:
	@echo "🔧 Formatting all Rust code..."
	@cargo fmt --all
	@echo "✅ Formatting complete"

# Check Rust formatting without modifying files
fmt-check:
	@echo "🔍 Checking Rust formatting..."
	@cargo fmt --all -- --check
	@echo "✅ Formatting check passed"
