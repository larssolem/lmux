// lmux KWin script - native window attach bridge.
// Schema version: 2.

"use strict";

function lmuxString(value) {
    if (value === undefined || value === null) {
        return "";
    }
    return value.toString();
}

function lmuxNumber(value) {
    return typeof value === "number" && isFinite(value) ? value : null;
}

function lmuxWindowList() {
    return typeof workspace.windowList === "function"
        ? workspace.windowList()
        : (typeof workspace.clientList === "function" ? workspace.clientList() : []);
}

function lmuxBackendWindowId(w) {
    var raw = "";
    if (w && w.internalId !== undefined && w.internalId !== null) {
        raw = lmuxString(w.internalId);
    } else if (w && w.windowId !== undefined && w.windowId !== null) {
        raw = lmuxString(w.windowId);
    } else if (w && w.uuid !== undefined && w.uuid !== null) {
        raw = lmuxString(w.uuid);
    }
    return raw.length > 0 ? "kwin:" + raw : "";
}

function lmuxWindowRecord(w) {
    var id = lmuxBackendWindowId(w);
    if (id.length === 0) {
        return null;
    }

    var desktop = w.desktop !== undefined ? lmuxString(w.desktop) : "";
    if (w.desktops !== undefined && w.desktops !== null && w.desktops.length > 0) {
        desktop = lmuxString(w.desktops[0]);
    }

    var output = "";
    if (w.output !== undefined && w.output !== null) {
        output = lmuxString(w.output.name !== undefined ? w.output.name : w.output);
    } else if (w.screen !== undefined && w.screen !== null) {
        output = lmuxString(w.screen);
    }

    return {
        backendWindowId: id,
        pid: lmuxNumber(w.pid),
        resourceClass: lmuxString(w.resourceClass),
        resourceName: lmuxString(w.resourceName),
        title: lmuxString(w.caption),
        workspace: desktop,
        output: output,
        normalWindow: typeof w.normalWindow === "boolean" ? w.normalWindow : null,
        skipTaskbar: typeof w.skipTaskbar === "boolean" ? w.skipTaskbar : null,
        skipSwitcher: typeof w.skipSwitcher === "boolean" ? w.skipSwitcher : null,
        specialWindow: typeof w.specialWindow === "boolean" ? w.specialWindow : null,
        desktopWindow: typeof w.desktopWindow === "boolean" ? w.desktopWindow : null,
        dock: typeof w.dock === "boolean" ? w.dock : null
    };
}

function lmuxCollectWindows() {
    var result = [];
    var windows = lmuxWindowList();
    for (var i = 0; i < windows.length; i++) {
        var record = lmuxWindowRecord(windows[i]);
        if (record !== null) {
            result.push(record);
        }
    }
    return result;
}

function lmuxLogWindow(label, client) {
    if (!client) {
        return;
    }
    var cls = client.resourceClass ? client.resourceClass.toString() : "?";
    var name = client.resourceName ? client.resourceName.toString() : "?";
    var caption = client.caption ? client.caption.toString() : "?";
    var pid = typeof client.pid === "number" ? client.pid : -1;
    print("lmux-dock [" + label + "] id=" + lmuxBackendWindowId(client) +
          " class=" + cls + " name=" + name + " pid=" + pid +
          " caption=" + caption);
}

if (typeof workspace.windowAdded !== "undefined") {
    workspace.windowAdded.connect(function (w) {
        lmuxLogWindow("added", w);
    });
    workspace.windowRemoved.connect(function (w) {
        lmuxLogWindow("removed", w);
    });
} else if (typeof workspace.clientAdded !== "undefined") {
    workspace.clientAdded.connect(function (c) {
        lmuxLogWindow("added", c);
    });
    workspace.clientRemoved.connect(function (c) {
        lmuxLogWindow("removed", c);
    });
}

print("lmux-dock: loaded (schema=2)");
