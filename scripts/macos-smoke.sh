#!/usr/bin/env bash
set -euo pipefail

echo "== lmux macOS smoke =="
echo "rustc: $(rustc --version)"
echo "cargo: $(cargo --version)"

required_zig="0.15.2"
zig_bin="${ZIG:-zig}"
if ! command -v "$zig_bin" >/dev/null 2>&1; then
  echo "missing Zig. Install Zig $required_zig or set ZIG=/path/to/zig" >&2
  exit 1
fi

zig_version="$("$zig_bin" version)"
echo "zig: $zig_version ($zig_bin)"
if [[ "${zig_version%%-*}" != "$required_zig" ]]; then
  echo "warning: vendored Ghostty expects Zig $required_zig (got $zig_version); continuing anyway" >&2
fi

echo "== protocol / portable crates =="
cargo test -p lmux-compositor
cargo test -p lmux-bus -p lmux-control -p lmux-pty
cargo test -p lmux-macos-helper

echo "== app compile =="
cargo check -p lmux

cat <<'SMOKE'
== manual GUI smoke ==
1. Start lmux and create anchors A and B.
2. Launch Chrome under A, complete any setup, then switch to B; A's Chrome window should minimize.
3. Launch another Chrome window under B and switch between A/B; only the active anchor's Chrome window should restore.
4. Repeat with IntelliJ: project chooser may appear first, but the real project window should replace it.
5. Close a tracked window and switch anchors; lmux should stop trying to restore the closed window.
SMOKE

echo "== done =="
