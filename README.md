# lmux

lmux collects terminal panes and ordinary app windows into work contexts. Start a
terminal, add the browser/editor/IDE windows that belong with it, then switch
contexts and bring the right windows back together.

It is a GUI multiplexer for developer workstations: tmux-style terminal control,
plus explicit window attach for native apps. Linux/KDE is the primary supported
desktop today; the macOS port is active and uses the same attach-first model.

## Status

v0.2 shipped on Linux/KDE; v0.3 is in planning. The macOS port is usable enough
for native terminal panes and explicit window attach work, but it is still
tracked as port work. [Capability specs](openspec/specs/) are the current
behavior contract. [Active change proposals](openspec/changes/) are proposals,
not shipped behavior. [Numbered ADRs](docs/adr/) are design history; older
Accepted ADRs may be superseded by the current specs or by later ADRs.

## Mental Model

```text
lmux window
├─ workspace: checkout-api
│  ├─ terminal: shell / agent / build
│  └─ app windows: browser, editor, IDE
└─ workspace: billing-fix
   ├─ terminal: shell / agent / logs
   └─ app windows: browser, editor, IDE
```

The implementation still uses precise internal names:

- **Pane**: a terminal pane inside the lmux window.
- **Anchor**: the terminal root for a work context. The first terminal is an
  anchor automatically.
- **Satellite**: a native GUI window explicitly attached to an anchor.
- **Session**: saved lmux state. Today this means terminal pane layout,
  working directories, and anchor metadata. Native GUI windows are attached to
  live anchors, but lmux does not promise to respawn arbitrary app windows from
  a saved session.

In the UI, think "workspaces" and "add window" first. The anchor/satellite terms
matter mostly when reading specs or using the CLI.

## Build

Prerequisites and platform notes live in [BUILD.md](BUILD.md). Short form for
contributors:

```sh
cargo build --release
```

For daily local use, prefer the installed launcher path because it also installs
the desktop integration lmux needs:

```sh
mise trust
mise install
mise run install:local
```

On Linux this registers a desktop entry and installs the KWin script. On macOS it
registers `~/Applications/lmux.app`.

## Platform Support

| Platform | Terminal panes | Native window attach | Notes |
|----------|----------------|----------------------|-------|
| KDE Plasma Wayland | Yes | Best supported | Uses KWin scripting for list, attach, hide/show, raise, and previews. |
| X11 | Yes | Best effort | Requires `xprop` and `xdotool`; exact behavior depends on the window manager. |
| Other Wayland compositors | Yes | Usually disabled | Terminal cockpit works; native attach needs a backend. |
| macOS | Yes | Active port | Uses Accessibility and CoreGraphics window ids; attach is per window, not per app bundle. |

lmux does not own monitor placement or window geometry for attached native
windows. Put windows where you want them; when you switch workspace anchors,
lmux hides/shows and raises the windows that belong to the active anchor.

## First Two Minutes: Linux/KDE

1. Install and register lmux:

   ```sh
   mise run install:local
   ```

2. Start lmux from the app launcher, or run:

   ```sh
   lmux
   ```

3. A single terminal opens. It is already the active workspace anchor.
4. Open a normal KDE window, such as Kate, your browser, or JetBrains.
5. In the lmux sidebar, click the link/add-window button. Choose the open window
   from the picker.
6. The selected window is attached to the active workspace. Switch workspaces
   with `Ctrl+B`, then `a`; the attached windows follow the active context.

Linux attach support is best on KDE Plasma Wayland through KWin. X11 is
best-effort when `xprop` and `xdotool` are installed. Other Wayland compositors
can run the terminal cockpit, but native app attach may be disabled until that
backend is implemented.

lmux does not currently provide a Linux program launcher. Open apps from your
desktop, shell, or app menu, then attach the exact window you want lmux to
manage.

## Daily Workflow

1. Start `lmux`; the first terminal is already the active workspace anchor.
2. Split terminal panes with `Ctrl+B -` or `Ctrl+B +`.
3. Open a normal app window from the desktop or shell.
4. Use the sidebar link/add-window button and choose the exact window to attach.
5. Create or cycle workspaces with `Ctrl+B a`; lmux keeps the active
   workspace's attached windows visible and in front.
6. Save named contexts with `lmux-cli session new <name>` and switch them with
   `Ctrl+B s` or `lmux-cli session open <name>`.

Session switching restores terminal pane layout, working directories, and anchor
metadata. Live native app windows are treated as live compositor state, not as
programs lmux can reliably relaunch later.

## First Two Minutes: macOS

1. Install dependencies and check the port prerequisites:

   ```sh
   mise trust
   mise install
   mise run doctor:macos
   ```

2. Build and register the local app:

   ```sh
   mise run install:local
   ```

3. Grant Accessibility permission when prompted, or request it explicitly:

   ```sh
   lmux --request-permissions
   ```

4. Start `lmux` from `~/Applications/lmux.app`, or run `lmux`.
5. Open a normal macOS app window.
6. In the lmux sidebar, click the link/add-window button and choose that window.

macOS uses explicit window ownership through CoreGraphics window ids. lmux does
not control every window from an app bundle; it only manages the exact windows
you attach. The old launcher flow is disabled on macOS. See
[docs/macos-port.md](docs/macos-port.md) for current port notes.

## Keybinds

lmux uses a tmux-style prefix: press `Ctrl+B`, then the command key. The window
title flashes `lmux [*]` while the prefix is armed, and it auto-disarms after
one second.

| Keybind | Action |
|---------|--------|
| `Ctrl+B` `-` | Split focused pane horizontally, new pane below |
| `Ctrl+B` `+` / `\|` / `\` | Split focused pane vertically, new pane right |
| `Ctrl+B` `x` | Close focused pane |
| `Ctrl+B` `]` / `o` / `n` | Cycle focus forward |
| `Ctrl+B` `[` / `p` | Cycle focus backward |
| `Ctrl+B` `a` | Cycle active workspace anchor |
| `Ctrl+B` `s` | Open the session switcher |
| `Ctrl+B` `m` | Rearrange panes |
| `Ctrl+B` `q` | Quit lmux, saving session state |
| `Ctrl+Shift+C` / `Ctrl+Shift+V` | Copy / paste in terminal panes |
| macOS: `Command+C` / `Command+V` | Copy / paste in terminal panes |
| `PageUp` / `PageDown` / mouse wheel | Scrollback in focused pane |

When an attached GUI window has focus, lmux lets key events pass through so the
native app keeps its own shortcuts.

If the clipboard contains an image, terminal paste writes it to an ephemeral PNG
under the runtime temp directory and pastes the absolute file path into the PTY.
This is for CLI tools that accept image paths; lmux does not inject raw image
bytes into the terminal.

## Configuration

User config lives at `$XDG_CONFIG_HOME/lmux/config.toml`, falling back to
`~/.config/lmux/config.toml`.

```toml
[general]
font_family = "JetBrains Mono"
font_size = 11
focus_mode = "click" # or "hover"

[keymap]
prefix = "ctrl+b"

[sidebar]
position = "left"
width = 280
collapsed_width = 48
collapsed = false
preview_enabled = true
preview_refresh_ms = 750
default_sort = "manual"

[[autodetect]]
name = "rust-build"
match = { command_contains = ["cargo build", "cargo test"] }
hide_on_session_close = true
```

The prefix key and general/sidebar settings are the supported user-facing
configuration surface today. The follower key table is compiled in.

## Companion CLI

Most first-run actions are available from the GUI. Shells and scripts can also
talk to a running lmux process over the bus socket:

```sh
lmux-cli status
lmux-cli pane list
lmux-cli anchor tag <uuid>
lmux-cli anchor pause <uuid>
lmux-cli session list
lmux-cli session new <name>
lmux-cli session rename <from> <to>
lmux-cli session delete <name>
lmux-cli session open <name>
lmux-cli satellite list-windows
lmux-cli satellite attach-window --backend kwin --backend-window-id <id>
lmux-cli satellite attach-window --backend x11 --backend-window-id <id>
lmux-cli satellite attach-focused
```

Use `./target/release/lmux-cli ...` from a build tree, or `lmux-cli ...` after
`mise run install:local`. Run `lmux-cli help` or `lmux-cli <sub> --help` for the
full surface.

Use the GUI for ordinary workspace and window management. Use the CLI when you
want scripts, diagnostics, or exact ids. `lmux-cli pane list` shows pane UUIDs
for `anchor tag`; sidebar popovers expose anchor UUIDs for pause/resume/hide.
`satellite list-windows` shows backend window ids for explicit attach. On macOS,
`satellite attach-focused` attaches the currently focused native window.

## Documentation

- [`BUILD.md`](BUILD.md): prerequisites, build, install, and troubleshooting
- [`docs/macos-port.md`](docs/macos-port.md): current macOS port notes
- [`openspec/specs/`](openspec/specs/): living capability specs
- [`openspec/changes/`](openspec/changes/): active change proposals
- [`docs/adr/`](docs/adr/): numbered architecture decisions
- [`docs/history/`](docs/history/): product brief, original brainstorm, v0.2 PRD

## License

Dual-licensed under MIT or Apache-2.0 at your option.
