# Agent terminal control

This document describes the CLI workflow agents can use before or alongside the
MCP adapter. The rule is simple: MCP tools and terminal commands use the same
lmux bus operations and the same cockpit-owned permissions.

## Discover workspaces

```sh
lmux-cli anchor list
lmux-cli --json anchor list
lmux-cli anchor active
```

`anchor list` returns workspace anchors only. Terminal panes/tabs and GUI
satellites are separate surfaces inside an anchor and are not added to the
anchor rail.

## Discover panes

```sh
lmux-cli pane list
```

The output contains each live pane UUID, owning anchor UUID when the pane
belongs to an anchor workspace or terminal tab stack, and best-effort current
working directory. Use pane UUIDs for transcript and input commands.

## Read terminal output

```sh
lmux-cli pane tail <pane-uuid> --lines 120
lmux-cli --json pane tail <pane-uuid> --lines 120
lmux-cli pane capture <pane-uuid> --since 42 --max-lines 80
```

Transcript output is captured from PTY-backed terminal panes before rendering.
GUI satellite panes do not expose transcript text.

## Send terminal input

```sh
lmux-cli pane send <pane-uuid> "q"
lmux-cli pane send <pane-uuid> "cargo test" --enter
```

Sending input can change program state. Agent workflows must prefer creating
their own pane for commands and should request user approval before sending
input to a pane in another anchor.

## Rename panes

```sh
lmux-cli pane rename <pane-uuid> "unit tests"
```

CLI rename is treated as a user-pinned title. Automatic agent rename requests
must not overwrite user-pinned titles.

## MCP setup

Build or install the MCP adapter so `lmux-mcp` is in `PATH`:

```sh
cargo install --path crates/lmux-mcp --force
```

Inspect local MCP availability:

```sh
lmux-cli mcp status
lmux-cli --json mcp status
```

Configure a known AI CLI client:

```sh
lmux-cli mcp install --client codex
lmux-cli mcp install --client claude
lmux-cli mcp install --client auto
```

Use `--dry-run` to print the client command without changing any client config:

```sh
lmux-cli mcp install --client codex --dry-run
lmux-cli mcp install --client claude --dry-run
```

For manual setup, print config snippets:

```sh
lmux-cli mcp print-config --format json
lmux-cli mcp print-config --format codex-toml
lmux-cli mcp print-config --format claude-project
```

`install` and `print-config` prefer `lmux-mcp` from `PATH`; if it is not in
`PATH`, they fall back to a `lmux-mcp` binary next to the running `lmux-cli`.

The adapter speaks MCP over stdio and exposes tools for anchor listing, pane
creation, transcript tail/capture, pane input, pane rename, native window
listing, GUI window attach, and GUI launch-and-attach. Each tool calls
`lmux_bus::Client` directly with the `lmux_mcp` client role. It does not shell
out through `lmux-cli` and it does not carry permissions that the CLI path lacks.

`satellite_launch` accepts `argv` plus optional `title_hint`, `app_hint`, and
`timeout_ms`. lmux starts the process, waits for a matching new native window or
the spawned PID, then uses the same exact-window attach grant as
`satellite_attach_window`.

Set `LMUX_AGENT_ID` and optionally `LMUX_AGENT_NAME` in the MCP client
environment to make grant prompts identify the requesting agent. When those
variables are absent, `lmux-mcp` uses a default `lmux-mcp` identity so
cross-anchor operations still flow through cockpit-owned grants.

## Security notes

- Terminal transcripts may contain secrets. Cross-anchor reads require an
  explicit cockpit grant when agent grants are enabled.
- `pane send` is stronger than read access because it injects bytes into a
  running PTY.
- GUI satellite attach requests must show the exact app/window and target
  anchor before ownership changes.
- lmux does not implement network telemetry for agent terminal control.
