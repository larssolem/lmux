#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "Skipping macOS Homebrew dependencies on $(uname -s)"
  exit 0
fi

if ! command -v brew >/dev/null 2>&1; then
  echo "Homebrew is required for macOS dependencies. Install it from https://brew.sh/" >&2
  exit 1
fi

brew bundle --file Brewfile
