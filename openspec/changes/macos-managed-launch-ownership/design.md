## Context

macOS app launch behavior varies by app. Some apps create a new process and window directly. Others forward launch requests to an existing process. Chrome and JetBrains can create transient setup/project chooser windows before a real work window exists. A single app process can own windows for several anchors.

The previous heuristic approach used app bundle snapshots and `System Events` window indices. That is not robust enough: indices are unstable, titles can be absent, and bundle-wide operations risk controlling unrelated user windows.

## Goals

- Preserve the user setup for lmux-launched apps across lmux restarts.
- Avoid touching the user's normal app profiles and existing app windows.
- Support multiple windows from the same app assigned to different anchors.
- Avoid guessing when ownership cannot be proven.
- Keep Linux behavior unchanged.

## Non-Goals

- Native macOS window embedding inside GTK.
- Claiming arbitrary existing user windows automatically.
- Per-anchor or per-launch app profiles as the default.
- Private macOS APIs or SIP-disabling techniques.

## Decisions

### D1 - Managed profile scope is per app, not per anchor

Each supported app gets one persistent lmux-managed profile:

```text
~/Library/Application Support/lmux/app-profiles/<app-key>/
```

For Chrome-family apps this maps to `--user-data-dir`. For JetBrains apps it maps to persistent `IDEA_PROPERTIES` paths. This avoids per-launch setup churn while keeping lmux-owned apps separate from the user's normal app profile.

Anchor ownership is not solved by profiles. It is solved by stable window identity.

### D2 - App profiles isolate lmux ownership; window identity separates anchors

The managed profile lets lmux classify a process/app instance as lmux-owned. Within that lmux-owned app, individual windows are mapped to anchors:

```text
window_id -> anchor_id
request_id -> pending launch intent
bundle_id/profile -> lmux-owned process/window set
```

Multiple windows from the same app profile can therefore belong to different anchors.

### D3 - Stable helper identity replaces `window_index`

The helper must expose a stable window identifier, preferably `CGWindowID` plus AX metadata. `window_index` can remain a debug field but must not be the ownership key.

Required helper window fields:

```text
window_id
pid
bundle_id
owner_name
title
profile_hint/process_hint when available
created_or_first_seen_at
```

### D4 - Launch tracking is a state machine

Each macOS launch request becomes a tracked intent:

```text
pending -> candidate -> primary -> closed | unmanaged
```

Transient setup/project windows may be candidates. A later better window for the same launch can replace the candidate as primary. Replacement updates the existing `request_id` mapping rather than adding duplicate anchor ownership.

### D5 - Anchor switch reconciles current state

On every anchor switch, lmux asks the helper for current lmux-owned windows and reconciles:

1. attach unassigned windows to pending launch requests when unambiguous;
2. drop destroyed windows from state;
3. minimize lmux-owned windows not assigned to the active anchor;
4. restore lmux-owned windows assigned to the active anchor.

The switch operation must not perform bundle-wide hide/minimize against apps with windows for multiple anchors.

### D6 - Ambiguity degrades to unmanaged

If the helper cannot prove a window belongs to a launch request or anchor, lmux leaves that window unmanaged. A future manual attach action can assign the focused lmux-owned window to the active anchor.

## Open Questions

- Which generic app families can safely receive managed profile paths beyond Chromium and JetBrains?
- Should manual attach be exposed first through CLI/bus, sidebar, or both?
- How long should a launch stay pending before becoming unmanaged?
