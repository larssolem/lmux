## 1. Product framing from review panel

- [ ] 1.1 Capture the current strength: lmux is not just a terminal mux; it groups terminal panes and native app windows into developer work contexts.
- [ ] 1.2 Preserve the explicit attach-first model: users attach exact native windows to workspaces, and lmux does not own monitor placement or geometry.
- [ ] 1.3 Keep the honest persistence contract: terminal layout, cwd, and anchor metadata can be restored; arbitrary native GUI state must not be promised as durable until proven.
- [ ] 1.4 Define the user-facing vocabulary for this work: prefer `workspace`, `window`, `add`, `move here`, and `restore`; keep `anchor`, `satellite`, UUIDs, and pane ids in advanced/CLI contexts.

## 2. Workspace profile model

- [ ] 2.1 Define a workspace profile schema that can describe terminal layout, working directories, anchor labels/groups, expected apps, and optional attach rules.
- [ ] 2.2 Separate durable state from live state in the schema: live window ids are observations, not long-term restore identifiers.
- [ ] 2.3 Add explicit restore confidence fields for native apps: `exact`, `likely`, `manual_attach_required`, or `unsupported`.
- [ ] 2.4 Add a GUI affordance for saving the current workspace as a named profile without requiring `lmux-cli session new <name>`.
- [ ] 2.5 Add a preview/confirmation step before restoring a profile that will terminate panes, hide windows, or launch external apps.

## 3. Native app restore risk handling

- [ ] 3.1 Treat native app restore as best-effort orchestration, not exact replay.
- [ ] 3.2 Document that some apps cannot be fully controlled at launch time: they may open multiple windows, close splash/login windows, restore their own previous session, or create windows after unpredictable delays.
- [ ] 3.3 Design a stabilization window for launched apps before lmux decides which windows are candidates for attach.
- [ ] 3.4 Add matching rules that can combine process id, app id/bundle id, window title, cwd, command line, and user confirmation instead of relying on one identifier.
- [ ] 3.5 Provide a manual reconciliation UI after restore: "These windows look related; attach, ignore, or move to another workspace."
- [ ] 3.6 Avoid promising Chrome-style tab restore. Restoring Chrome is not just launching Chrome; lmux may not know or control which profile, window, and tabs should appear.
- [ ] 3.7 For browsers, consider explicit URL intents or browser-profile commands as a separate mechanism from generic native window restore.
- [ ] 3.8 For IDEs/editors, prefer project/path-based launch intents where possible, then attach the resulting stable window after it appears.
- [ ] 3.9 Record failed/ambiguous restore attempts in workspace history so the next restore can guide the user instead of silently doing the wrong thing.

## 4. Restore UX

- [ ] 4.1 Show a restore plan before execution: terminals to create, apps to launch, windows expected, and items that require manual attach.
- [ ] 4.2 During restore, show per-item states: `launching`, `waiting for window`, `matched`, `ambiguous`, `manual`, `failed`.
- [ ] 4.3 Let users continue working while native app reconciliation is still pending.
- [ ] 4.4 Make restore latest-wins: if the user switches workspace/profile during restore, stale app matching and visibility work must not take over the new active workspace.
- [ ] 4.5 Add a safe cancel path for in-progress restore that stops lmux-owned pending work without killing unrelated user windows.

## 5. Platform and compositor readiness

- [ ] 5.1 Add KWin health diagnostics for native attach, preview, hide/show, and raise support.
- [ ] 5.2 Surface attach/restore capability clearly in the sidebar when the compositor backend cannot support native window control.
- [ ] 5.3 Keep wlroots/Hyprland/Sway support as a separate backend effort; do not let workspace restore depend on unimplemented compositor capabilities.
- [ ] 5.4 Add manual smoke recipes for restoring profiles with Chrome, a JetBrains IDE, a plain GTK app, and a terminal-only workspace.

## 6. CLI and automation

- [ ] 6.1 Add JSON output for profile/session status so scripts can inspect restore state without parsing human text.
- [ ] 6.2 Add commands for profile save, profile restore, restore dry-run, and restore status.
- [ ] 6.3 Add event subscription or polling support for restore progress if the bus becomes a public plugin surface.
- [ ] 6.4 Require scoped/capability-based bus access before exposing restore-launch operations to external plugins.

## 7. Documentation and onboarding

- [ ] 7.1 Add a "What is saved vs restored vs best-effort" table to README.
- [ ] 7.2 Add a first-run verification path: install, start lmux, run `lmux-cli status`, attach one window, switch workspace, and explain expected behavior.
- [ ] 7.3 Explain that app restore is different from window attach: attach controls an existing exact window; restore may need to launch an app and then infer which resulting window belongs to the workspace.
- [ ] 7.4 Add examples for browser and IDE workflows that set realistic expectations instead of implying exact full-session replay.

## 8. Product polish follow-ups

- [ ] 8.1 Align UI copy with the workspace model and hide internal anchor/satellite language from ordinary flows.
- [ ] 8.2 Make the session/profile switcher a proper modal or command palette with a clear title, empty state, keyboard handling, and restore risk indicators.
- [ ] 8.3 Decide whether the launcher is a supported part of workspace restore or an experimental/deprecated path, then align README, specs, and code comments.
- [ ] 8.4 Introduce small design tokens for status colors, row spacing, radius, dim text, warning, and success states.
