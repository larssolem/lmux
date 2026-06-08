#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
os="$(uname -s)"

echo "Installing lmux locally for $os"

cargo install --path "$repo_root/crates/lmux" --force
cargo install --path "$repo_root/crates/lmux-cli" --force
cargo install --path "$repo_root/crates/lmux-mcp" --force

if [[ -n "${CARGO_INSTALL_ROOT:-}" ]]; then
  lmux_bin="$CARGO_INSTALL_ROOT/bin/lmux"
elif [[ -n "${CARGO_HOME:-}" ]]; then
  lmux_bin="$CARGO_HOME/bin/lmux"
else
  lmux_bin="$HOME/.cargo/bin/lmux"
fi

if [[ ! -x "$lmux_bin" ]]; then
  echo "Installed lmux binary was not found at $lmux_bin" >&2
  exit 1
fi

case "$os" in
  Darwin)
    app_dir="$HOME/Applications/lmux.app"
    contents_dir="$app_dir/Contents"
    macos_dir="$contents_dir/MacOS"
    mkdir -p "$macos_dir"

    cat >"$contents_dir/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>en</string>
  <key>CFBundleDisplayName</key>
  <string>lmux</string>
  <key>CFBundleExecutable</key>
  <string>lmux</string>
  <key>CFBundleIdentifier</key>
  <string>no.jpro.lmux</string>
  <key>CFBundleName</key>
  <string>lmux</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>0.1.0-dev</string>
  <key>CFBundleVersion</key>
  <string>0.1.0-dev</string>
  <key>LSMinimumSystemVersion</key>
  <string>13.0</string>
  <key>NSHighResolutionCapable</key>
  <true/>
  <key>NSAppleEventsUsageDescription</key>
  <string>lmux uses System Events to list windows that can be attached to anchors.</string>
</dict>
</plist>
PLIST

    cp "$lmux_bin" "$macos_dir/lmux"
    chmod +x "$macos_dir/lmux"

    lsregister="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"
    if [[ -x "$lsregister" ]]; then
      "$lsregister" -f "$app_dir" >/dev/null 2>&1 || true
    fi
    echo "Installed app launcher: $app_dir"
    if command -v open >/dev/null 2>&1; then
      echo "Requesting macOS Accessibility permission for $app_dir"
      open "$app_dir" --args --request-permissions >/dev/null 2>&1 || true
    fi
    ;;
  Linux)
    data_home="${XDG_DATA_HOME:-$HOME/.local/share}"
    applications_dir="$data_home/applications"
    kwin_dir="$data_home/lmux/kwin"
    desktop_file="$applications_dir/no.jpro.lmux.desktop"
    mkdir -p "$applications_dir"
    mkdir -p "$kwin_dir"
    cp "$repo_root/share/lmux/kwin/lmux-dock.js" "$kwin_dir/lmux-dock.js"
    chmod 0644 "$kwin_dir/lmux-dock.js"

    cat >"$desktop_file" <<DESKTOP
[Desktop Entry]
Type=Application
Name=lmux
Comment=GUI multiplexer for terminal panes and app windows
Exec=$lmux_bin
Icon=utilities-terminal
Terminal=false
Categories=System;TerminalEmulator;Utility;
StartupNotify=true
StartupWMClass=no.jpro.lmux
X-KDE-DBUS-Restricted-Interfaces=org.kde.KWin.ScreenShot2
DESKTOP

    chmod 0644 "$desktop_file"
    if command -v update-desktop-database >/dev/null 2>&1; then
      update-desktop-database "$applications_dir" >/dev/null 2>&1 || true
    fi
    echo "Installed desktop launcher: $desktop_file"
    echo "Installed KWin script: $kwin_dir/lmux-dock.js"
    ;;
  *)
    echo "Installed lmux binary: $lmux_bin"
    echo "No app launcher installer is defined for $os."
    ;;
esac
