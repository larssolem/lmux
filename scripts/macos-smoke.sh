#!/usr/bin/env bash
set -euo pipefail

echo "== lmux macOS smoke =="
echo "rustc: $(rustc --version)"
echo "cargo: $(cargo --version)"

zig_bin="${ZIG:-zig}"
if ! command -v "$zig_bin" >/dev/null 2>&1; then
  echo "missing Zig. Install Zig 0.15.2 or set ZIG=/path/to/zig-0.15.2" >&2
  exit 1
fi

zig_version="$("$zig_bin" version)"
echo "zig: $zig_version ($zig_bin)"
if [[ "$zig_version" != 0.15.2* ]]; then
  echo "warning: vendored Ghostty currently expects Zig 0.15.2; continuing anyway" >&2
fi

echo "== protocol / portable crates =="
cargo test -p lmux-compositor
cargo test -p lmux-bus -p lmux-control -p lmux-pty

echo "== app compile =="
cargo check -p lmux

echo "== done =="
