// lmux KWin script — Path A satellite docking bridge.
// Schema version: 1.
//
// Runtime install target: ~/.local/share/kwin/scripts/lmux/main.js
// Installed by lmux-compositor::ensure_script_loaded on first run.
//
// What this script does in v0.2:
//
//   * Listens for `workspace.windowAdded` and logs the new window's
//     resourceClass / caption / pid. This makes the script observable in
//     `journalctl --user -t kwin_scripting` while the Rust side proves out
//     the spawn path.
//   * Reads /proc/<pid>/environ (via KWin's built-in `readConfig` is not
//     available in scripts, so we shell out by calling into `/proc`
//     through `callDBus` workarounds). KWin scripts cannot open files
//     directly, so environ correlation runs on the Rust side in v0.3.
//
// What v0.3 will add:
//
//   * A JSON state file at $XDG_RUNTIME_DIR/lmux/satellites.json tracking
//     { LMUX_SATELLITE_ID -> { internalId, class, pid } } so the cockpit
//     can issue `satellite.map` events back to its own bus.
//   * Geometry / detach / reattach: when the cockpit emits `set_geometry`
//     it will call into this script via `callDBus` on a secondary object
//     path registered by a script-mode Plasma applet.
//
// For v0.2 the script's only job is to be loadable + log events. That's
// enough to satisfy `KwinCompositor::ensure_script_loaded` and to give us
// diagnostics when a real satellite window appears.

"use strict";

function logWindow(label, client) {
    if (!client) {
        return;
    }
    var cls = client.resourceClass ? client.resourceClass.toString() : "?";
    var name = client.resourceName ? client.resourceName.toString() : "?";
    var caption = client.caption ? client.caption.toString() : "?";
    var pid = typeof client.pid === "number" ? client.pid : -1;
    print("lmux-dock [" + label + "] class=" + cls + " name=" + name +
          " pid=" + pid + " caption=" + caption);
}

// KWin 5.27+: workspace.windowAdded / windowRemoved. Older KWin exposes
// clientAdded / clientRemoved — we try both so the script loads on Plasma
// 5 and 6.
if (typeof workspace.windowAdded !== "undefined") {
    workspace.windowAdded.connect(function (w) { logWindow("added", w); });
    workspace.windowRemoved.connect(function (w) { logWindow("removed", w); });
} else if (typeof workspace.clientAdded !== "undefined") {
    workspace.clientAdded.connect(function (c) { logWindow("added", c); });
    workspace.clientRemoved.connect(function (c) { logWindow("removed", c); });
}

print("lmux-dock: loaded (schema=1)");
