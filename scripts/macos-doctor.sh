#!/usr/bin/env bash
set -u

minimum_zig="0.15.2"
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

check_pkg_config() {
  local module="$1"
  local hint="$2"
  if command -v pkg-config >/dev/null 2>&1; then
    if version="$(pkg-config --modversion "$module" 2>/dev/null)"; then
      ok "$module pkg-config version: $version"
    else
      fail "$module pkg-config metadata not found. $hint"
    fi
  fi
}

if command -v zig >/dev/null 2>&1; then
  zig_version="$(zig version 2>/dev/null || true)"
  # Numeric compare of MAJOR.MINOR.PATCH against minimum_zig.
  if [[ -n "$zig_version" ]] && printf '%s\n%s\n' "$minimum_zig" "${zig_version%%-*}" \
      | sort -V -C 2>/dev/null; then
    ok "zig version is $zig_version (>= $minimum_zig)"
  else
    fail "expected zig >= $minimum_zig, got ${zig_version:-unknown}. Run: mise install"
  fi
fi

if command -v rustc >/dev/null 2>&1; then
  ok "$(rustc --version)"
fi

check_pkg_config "gtk4" "Run: brew bundle"
check_pkg_config "graphene-gobject-1.0" "Run: brew bundle; graphene-sys requires this module"

sdk_is_zig_compatible() {
  local sdk="$1"
  local tbd_rel tbd
  [[ -d "$sdk/usr/lib" ]] || return 1
  for tbd_rel in usr/lib/libSystem.tbd usr/lib/system/libdispatch.tbd usr/lib/system/libsystem_c.tbd; do
    tbd="$sdk/$tbd_rel"
    if [[ -f "$tbd" ]] && grep -qE '\barm64e-macos\b' "$tbd" && ! grep -qE '\barm64-macos\b' "$tbd"; then
      return 1
    fi
  done
  return 0
}

# macOS 26+ SDKs can omit arm64-macos slices that Zig 0.15.2 needs when
# linking its native build runner. The lmux build script can use a compatible
# installed CommandLineTools SDK through a local xcrun shim, without modifying
# Apple's SDK files.
if [[ "$host_os" == "Darwin" ]] && command -v xcrun >/dev/null 2>&1; then
  sdk_path="$(xcrun --sdk macosx --show-sdk-path 2>/dev/null || true)"
  if [[ -n "$sdk_path" ]] && sdk_is_zig_compatible "$sdk_path"; then
    ok "selected macOS SDK is compatible with Zig 0.15.2: $sdk_path"
  else
    compatible_sdk=""
    for candidate in /Library/Developer/CommandLineTools/SDKs/MacOSX*.sdk; do
      if [[ -d "$candidate" ]] && sdk_is_zig_compatible "$candidate"; then
        compatible_sdk="$candidate"
      fi
    done
    if [[ -n "$compatible_sdk" ]]; then
      ok "compatible fallback macOS SDK for Zig 0.15.2: $compatible_sdk"
    else
      fail "no Zig-compatible macOS SDK found. Install Command Line Tools that include MacOSX15.x.sdk or upgrade the vendored Ghostty/Zig toolchain."
    fi
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
