# lmux agent CLI skill draft

Use this draft as the basis for a future agent skill when MCP is unavailable or
not desired.

## Instructions

When working inside lmux:

1. Discover the current workspace.

   ```sh
   lmux-cli --json anchor active
   ```

2. List panes before reading or controlling terminals.

   ```sh
   lmux-cli --json pane list
   ```

3. For long-running commands, prefer creating or using an agent-owned pane with
   `lmux-cli pane new`.

4. Read terminal output through transcript commands instead of scraping the
   screen.

   ```sh
   lmux-cli --json pane tail <pane-uuid> --lines 120
   lmux-cli --json pane capture <pane-uuid> --since <sequence>
   ```

5. Do not send input to a pane in another anchor unless lmux has shown a grant
   prompt and the user allowed it.

6. If renaming a pane, choose short operational names such as `tests`, `server`,
   `api logs`, or `playwright`. Do not overwrite user-pinned names.

7. For GUI windows, list candidates and request exact-window attach through
   lmux. When launching a new GUI app through MCP, provide `title_hint` or
   `app_hint` so lmux can avoid attaching an unrelated pre-existing window.

## Expected behavior

- Prefer `--json` for machine parsing.
- Treat errors from `lmux-cli` as authoritative cockpit decisions.
- Keep user-visible names concise.
- Explain why cross-anchor access is needed before requesting it.
