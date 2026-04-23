# lmux

**GUI multiplexer for Linux** — bundle terminal panes and docked GUI apps into work contexts, switch context with one keystroke. Spiritual successor to macOS `cmux`, built on libghostty + GTK4 with a KWin-scripted docking layer.

## Status

v0.2 shipped; v0.3 in planning. [Capability specs](openspec/specs/) describe what the system does today, nine capabilities in total: `terminal-core`, `sessions`, `anchors`, `sidebar`, `satellites`, `compositor-control`, `bus-ipc`, `config`, `observability`. [Active change proposals](openspec/changes/) cover what's next; [numbered ADRs](docs/adr/) carry the *why* behind each decision.

## The mental model

```
┌─────────────────────────────── lmux cockpit ───────────────────────────────┐
│                                                                            │
│  ╭── pane ──╮ ╭── pane ──╮   ╭── satellite ─────────────────────────╮       │
│  │ shell    │ │ cargo    │   │ Kate / browser / JetBrains           │       │
│  │          │ │ watch  ⚓ │   │   (GUI app docked into the cockpit)   │       │
│  ╰──────────╯ ╰──────────╯   ╰───────────────────────────────────────╯     │
│                                                                            │
│   sidebar: session list · anchors · satellites                             │
└────────────────────────────────────────────────────────────────────────────┘
```

- **Panes** host shells inside a single cockpit window (the tmux part)
- **Anchors** (⚓) are tagged panes whose lifecycle you care about — they survive close, pause via `SIGSTOP`, capture crash output in a scrollback ring
- **Satellites** are full GUI apps docked into the cockpit through compositor scripting (KWin today; wlroots/Hyprland in v0.3)
- **Sessions** persist the full tree + anchors + satellite metadata so you can quit and come back

The v0.3 direction layers a **workflow switcher** on top: bind a key per workflow; switching swaps the whole window set.

## Build

Prereqs and build steps live in [BUILD.md](BUILD.md). Short form:

```sh
cargo build --release
```

## Run

```sh
./target/release/lmux
```

A single pane launches with your `$SHELL`.

## Keybinds (v0.2)

lmux uses a tmux-style **prefix**: press `Ctrl+B`, then the command key. Rationale — Super-based shortcuts clash with KDE's global window-management bindings (Super+Q, Super+W, Super+[]). The window title flashes `lmux [◆]` while the prefix is armed; it auto-disarms after 1 s.

| Keybind | Action |
|---------|--------|
| `Ctrl+B` `-` | Split focused pane horizontally (new pane below) |
| `Ctrl+B` `\|` / `Ctrl+B` `\` / `Ctrl+B` `+` | Split focused pane vertically (new pane right) |
| `Ctrl+B` `x` | Close focused pane |
| `Ctrl+B` `]` / `Ctrl+B` `o` / `Ctrl+B` `n` | Cycle focus forward |
| `Ctrl+B` `[` / `Ctrl+B` `p` | Cycle focus backward |
| `Ctrl+B` `a` | Mark focused pane as anchor |
| `Ctrl+B` `q` | Quit lmux (clean shutdown, saves session) |
| `Ctrl+Shift+C` / `Ctrl+Shift+V` | Copy / paste (no prefix — standard terminal) |
| `PageUp` / `PageDown` / mouse wheel | Scrollback in focused pane |

## Companion CLI — `lmux-cli`

Shells and scripts talk to a running cockpit over the bus socket.

```sh
lmux-cli status                     # cockpit snapshot: pid, version, counts
lmux-cli mark-anchor                # promote the current pane to anchor
lmux-cli pane list                  # inventory of pane UUIDs
lmux-cli anchor tag <uuid>          # tag a pane as anchor
lmux-cli anchor pause <uuid>        # SIGSTOP the backing process
lmux-cli session list / new / attach / kill
lmux-cli satellite open <app> ...   # dock a GUI app to the focused pane
```

Run `lmux-cli help` / `lmux-cli <sub> --help` for the full surface.

## Documentation

- [`openspec/specs/`](openspec/specs/) — living capability specs
- [`openspec/changes/`](openspec/changes/) — active change proposals
- [`docs/adr/`](docs/adr/) — numbered architecture decisions
- [`docs/history/`](docs/history/) — product brief, original brainstorm, v0.2 PRD, e2e test strategy

## License

Dual-licensed under MIT or Apache-2.0 at your option.
