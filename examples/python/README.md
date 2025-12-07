# Python Sandbox Example

This simple script exercises the Boxlite Python SDK once the extension module has
been built locally.

## Prerequisites

1. Install [`maturin`](https://www.maturin.rs/) into your Python environment:
   ```bash
      python3 -m venv .venv
   source .venv/bin/activate
   python3 -m pip install maturin  # or use pipx install maturin
   ```
2. Run the bootstrap helper (on macOS it installs Homebrew `libkrun`/`libkrunfw` and stages
   the dylibs; on Linux it builds them from source into `build/`):
   ```bash
   ../../scripts/bootstrap.sh
   ```
3. Build the workspace and the Python extension:
   ```bash
   ../../scripts/build.sh --with-python
   ```
   (Use `--skip-bootstrap` on subsequent runs.)

## Run the Example

```bash
python examples/python/hello.py
```

The current libkrun engine is a host fallback: it runs the requested command on the host
interpreter (e.g., `python3 -c ...`) and streams stdout/stderr back through the SDK. Once
native libkrun execution is implemented this example will launch inside a VM automatically.
