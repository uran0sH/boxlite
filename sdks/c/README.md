# BoxLite C SDK

C bindings for the BoxLite runtime, providing a stable C API for integrating BoxLite into C/C++
applications.

## Overview

The C SDK provides:

- C-compatible FFI bindings (`cdylib`, `staticlib`)
- Auto-generated header file (`include/boxlite.h`)
- JSON-based API to avoid ABI compatibility issues
- Streaming output support via callbacks

## Building

```bash
# From repository root
cargo build --release -p boxlite-c

# Outputs:
# - target/release/libboxlite.{dylib,so}     (shared library)
# - target/release/libboxlite.a              (static library)
# - sdks/c/include/boxlite.h                 (auto-generated header)
```

## Installation

### Option 1: Direct Linking (Development)

```bash
# Copy library and header to your project
cp target/release/libboxlite.{dylib,so} /path/to/your/project/lib/
cp sdks/c/include/boxlite.h /path/to/your/project/include/

# Compile your program
gcc -I/path/to/include -L/path/to/lib -lboxlite your_program.c -o your_program
```

### Option 2: System Installation (Future)

```bash
# TODO: Add installation target
make install  # Installs to /usr/local/{lib,include}
```

### Option 3: CMake (Recommended)

See `examples/c/CMakeLists.txt` for a complete example.

```cmake
set(BOXLITE_ROOT "/path/to/boxlite")
set(BOXLITE_INCLUDE_DIR "${BOXLITE_ROOT}/sdks/c/include")
set(BOXLITE_LIB_DIR "${BOXLITE_ROOT}/target/release")

include_directories(${BOXLITE_INCLUDE_DIR})
target_link_libraries(your_target ${BOXLITE_LIB_DIR}/libboxlite.dylib)
```

## Quick Start

```c
#include <stdio.h>
#include "boxlite.h"

void output_callback(const char* text, int is_stderr, void* user_data) {
    printf("%s", text);
}

int main() {
    char* error = NULL;

    // Create runtime
    BoxliteRuntime* runtime = boxlite_runtime_new(NULL, &error);
    if (!runtime) {
        fprintf(stderr, "Error: %s\n", error);
        boxlite_free_string(error);
        return 1;
    }

    // Create box with Alpine Linux
    const char* options = "{\"image\":{\"Reference\":\"alpine:3.19\"}}";
    BoxHandle* box = boxlite_create_box(runtime, options, &error);
    if (!box) {
        fprintf(stderr, "Error: %s\n", error);
        boxlite_free_string(error);
        boxlite_runtime_free(runtime);
        return 1;
    }

    // Execute command
    const char* args = "[\"-la\"]";
    int exit_code = boxlite_execute(box, "/bin/ls", args,
                                      output_callback, NULL, &error);

    // Cleanup
    boxlite_shutdown_box(box, &error);
    boxlite_runtime_free(runtime);

    return exit_code;
}
```

## API Reference

### Runtime Management

```c
// Get BoxLite version
const char* boxlite_version();

// Create runtime instance
BoxliteRuntime* boxlite_runtime_new(
    const char* home_dir,      // NULL for default (~/.boxlite)
    char** out_error           // Output: error message (must free)
);

// Free runtime instance
void boxlite_runtime_free(BoxliteRuntime* runtime);
```

### Box Management

```c
// Create a new box
BoxHandle* boxlite_create_box(
    BoxliteRuntime* runtime,
    const char* options_json,  // JSON-encoded BoxOptions
    char** out_error
);

// Execute command in box
int boxlite_execute(
    BoxHandle* handle,
    const char* command,
    const char* args_json,     // JSON array: ["arg1", "arg2"]
    void (*callback)(const char* text, int is_stderr, void* user_data),
    void* user_data,           // Passed to callback
    char** out_error
);

// Shutdown box
int boxlite_shutdown_box(BoxHandle* handle, char** out_error);
```

### Memory Management

```c
// Free error strings returned by BoxLite
void boxlite_free_string(char* str);
```

## JSON API Schema

### BoxOptions

```json
{
  "image": {
    "Reference": "alpine:3.19"
    // OCI images reference
  },
  "working_dir": "/workspace",
  // Optional
  "env": {
    // Optional
    "KEY": "value"
  }
}
```

### Command Arguments

```json
[
  "arg1",
  "arg2",
  "arg3"
]
```

## Examples

See `../../examples/c/` for complete working examples:

- `execute.c` - Basic command execution with streaming output
- `CMakeLists.txt` - CMake build configuration

## Error Handling

All functions that can fail return error messages via `out_error` parameter:

```c
char* error = NULL;
BoxliteRuntime* runtime = boxlite_runtime_new(NULL, &error);
if (!runtime) {
    fprintf(stderr, "Failed: %s\n", error);
    boxlite_free_string(error);  // MUST free error string
    return 1;
}
```

**Important:** Always free error strings with `boxlite_free_string()`.

## Thread Safety

- `BoxliteRuntime` is thread-safe (uses internal async runtime)
- `BoxHandle` should not be shared across threads
- Callbacks are invoked on the calling thread

## Platform Support

- **macOS**: arm64, x86_64 (requires Hypervisor.framework)
- **Linux**: x86_64, aarch64 (requires KVM)

## Architecture

The C SDK is a thin wrapper around the Rust `boxlite` crate:

```
sdks/c/src/lib.rs
  ↓ (re-export)
boxlite/src/ffi.rs
  ↓ (wraps)
boxlite/src/runtime/
```

- Built as separate crate to produce `cdylib`/`staticlib`
- Header auto-generated from Rust code using `cbindgen`
- Maintains same functionality as Rust API

## Development

### Rebuilding Header

The header is auto-generated during build. To regenerate:

```bash
cargo build -p boxlite-c
# Outputs: sdks/c/include/boxlite.h
```

### Adding New Functions

1. Add function to `boxlite/src/ffi.rs` with `#[no_mangle]` and `extern "C"`
2. Rebuild C SDK: `cargo build -p boxlite-c`
3. Header is automatically updated

### cbindgen Configuration

Header generation is configured in `build.rs` using cbindgen's builder API. No separate config file
is needed.

## License

Apache-2.0
