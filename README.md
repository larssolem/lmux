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
tracked as port work. [Capability specs](openspec/specs/) describe what the
system does today. [Active change proposals](openspec/changes/) cover what is
next, and [numbered ADRs](docs/adr/) carry the design decisions.

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
- **Session**: saved lmux state, including panes, anchors, and attached windows.

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
```

Use `./target/release/lmux-cli ...` from a build tree, or `lmux-cli ...` after
`mise run install:local`. Run `lmux-cli help` or `lmux-cli <sub> --help` for the
full surface.

## Documentation

- [`BUILD.md`](BUILD.md): prerequisites, build, install, and troubleshooting
- [`docs/macos-port.md`](docs/macos-port.md): current macOS port notes
- [`openspec/specs/`](openspec/specs/): living capability specs
- [`openspec/changes/`](openspec/changes/): active change proposals
- [`docs/adr/`](docs/adr/): numbered architecture decisions
- [`docs/history/`](docs/history/): product brief, original brainstorm, v0.2 PRD

## License

Dual-licensed under MIT or Apache-2.0 at your option.
