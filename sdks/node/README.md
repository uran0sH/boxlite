# Boxlite JavaScript SDK (Draft)

This package will expose a Node.js API backed by `napi-rs` bindings to the
Boxlite runtime. Implementation is pending:

- Build `sdks/node/native` using `@napi-rs/cli`.
- Provide TypeScript definitions mirroring the Rust API (`Sandbox`, `ExecutionStream`).
- Handle async streaming of stdout/stderr from sandboxes.

Current scripts are placeholders so the directory structure exists for future work.
