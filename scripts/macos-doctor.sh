#!/usr/bin/env bash
set -u

expected_zig="0.15.2"
failures=0
warnings=0

ok() {
  printf 'ok   %s\n' "$1"
}

warn() {
  warnings=$((warnings + 1))
  printf 'warn %s\n' "$1"
}

fail() {
  failures=$((failures + 1))
  printf 'fail %s\n' "$1"
}

require_cmd() {
  local name="$1"
  local hint="$2"
  if command -v "$name" >/dev/null 2>&1; then
    ok "$name found: $(command -v "$name")"
  else
    fail "$name not found. $hint"
  fi
}

printf 'lmux macOS doctor\n'
printf '=================\n'

host_os="$(uname -s 2>/dev/null || printf unknown)"
if [[ "$host_os" == "Darwin" ]]; then
  ok "running on macOS"
else
  fail "expected macOS/Darwin host, got $host_os"
fi

if command -v xcode-select >/dev/null 2>&1; then
  if xcode_path="$(xcode-select -p 2>/dev/null)"; then
    ok "Xcode command line tools: $xcode_path"
  else
    fail "Xcode command line tools not selected. Run: xcode-select --install"
  fi
else
  fail "xcode-select not found. Run: xcode-select --install"
fi

require_cmd brew "Install Homebrew from https://brew.sh/"
require_cmd rustc "Run: mise install"
require_cmd cargo "Run: mise install"
require_cmd zig "Run: mise install"
require_cmd pkg-config "Run: brew bundle"

if command -v zig >/dev/null 2>&1; then
  zig_version="$(zig version 2>/dev/null || true)"
  if [[ "$zig_version" == "$expected_zig" ]]; then
    ok "zig version is $expected_zig"
  else
    fail "expected zig $expected_zig, got ${zig_version:-unknown}. Run: mise install"
  fi
fi

if command -v rustc >/dev/null 2>&1; then
  ok "$(rustc --version)"
fi

if command -v pkg-config >/dev/null 2>&1; then
  if gtk_version="$(pkg-config --modversion gtk4 2>/dev/null)"; then
    ok "gtk4 pkg-config version: $gtk_version"
  else
    fail "gtk4 pkg-config metadata not found. Run: brew bundle"
  fi
fi

if command -v brew >/dev/null 2>&1 && [[ -f Brewfile ]]; then
  if brew bundle check --file Brewfile >/dev/null 2>&1; then
    ok "Brewfile dependencies are installed"
  else
    warn "Brewfile dependencies are missing. Run: brew bundle"
  fi
fi

if command -v cargo >/dev/null 2>&1; then
  if cargo metadata --format-version 1 --no-deps >/dev/null 2>&1; then
    ok "cargo metadata succeeds"
  else
    fail "cargo metadata failed"
  fi
fi

printf '\n'
if ((failures == 0)); then
  if ((warnings == 0)); then
    printf 'macOS doctor passed.\n'
  else
    printf 'macOS doctor passed with %d warning(s).\n' "$warnings"
  fi
  exit 0
fi

printf 'macOS doctor failed with %d failure(s) and %d warning(s).\n' "$failures" "$warnings"
exit 1
