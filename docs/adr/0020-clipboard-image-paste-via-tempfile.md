# ADR-0020: Clipboard image paste via tempfile + path injection

- Status: Accepted
- Date: 2026-04-23
- Deciders: Lars
- Blocks: v0.2 (FR around AI-tooling ergonomics)
- Depends on: ADR-0001 (GTK4 + libghostty), ADR-0002 (cockpit owns IPC)

## Context

The Anthropic / OpenAI / Google CLIs (`claude`, `codex`, `gemini`) accept image inputs but only by **path** — they have no native protocol for receiving raw image bytes over a TTY. Users routinely have an image on their X11/Wayland clipboard (a screenshot, something dragged from a browser, an artboard from a design tool) and want to attach it to a conversation without first writing it to disk by hand.

The terminal layer (libghostty + xterm-style escape sequences) does have one image-on-the-wire mechanism: the iTerm2 inline image protocol. But that displays the image *in the terminal*; it does not deliver bytes to the running program in a form a CLI tool can consume. Sixel and Kitty graphics have the same display-only property. There is no standard for "deliver this image to the foreground process as data."

So the cockpit has to bridge: detect that the clipboard holds an image, materialize it as a file the foreground program can open, and tell the program where the file is. The "tell the program" part is constrained to what a TTY can carry — i.e., text typed into the PTY via bracketed paste.

## Decision

When the user invokes paste (`Ctrl+B ]` or platform paste shortcut), `request_paste_from_clipboard` runs the following decision:

1. Probe `gdk::Clipboard::formats().mime_types()`.
2. **If any MIME starts with `image/`**, read the clipboard as a `gdk::Texture`, save it as PNG to an ephemeral file, and inject the **absolute path** into the PTY via bracketed paste (`\x1b[200~<path>\x1b[201~`).
3. **Otherwise**, fall back to the existing text-paste path (read text, inject via bracketed paste).

### File location and lifecycle

```
$XDG_RUNTIME_DIR/lmux/pastes/paste-<pid>-<unix-millis>-<n>.png
```

Fallback when `XDG_RUNTIME_DIR` is unset: `/tmp/lmux-pastes-<uid>/paste-...png` with `0700` permissions on the parent directory.

- `XDG_RUNTIME_DIR` is tmpfs on every modern Linux desktop session and is wiped on logout — exactly the right TTL for a clipboard scratch buffer.
- Per-pid + monotonic counter prevents collisions when the user pastes the same image into multiple panes / multiple lmux processes.
- Always PNG: lossless, universally supported, what every CLI image consumer expects.

### What is *not* injected

- No raw image bytes over the wire (no Sixel, no iTerm2, no Kitty graphics). The CLI tool we're feeding doesn't understand any of those, and the terminal grid would scroll past the image anyway.
- No file deletion by the cockpit. The runtime dir is tmpfs and self-cleans on logout; deleting on paste-completion would race with whatever the CLI is doing with the file.

## Alternatives considered

- **Inline iTerm2 image protocol injection.** Rejected: shows the image in the terminal but the foreground program never sees it — solves the wrong problem.
- **Spawn a small "image attach" prompt** (file dialog, paste pad, etc.) and inject the chosen path. Rejected: extra UX surface for a one-shot operation; the clipboard is already the staging area the user picked.
- **Heuristic: only inject path if the foreground command appears to be `claude`/`codex`/etc.** Rejected: requires inspecting the PTY child's argv (racy, and the user may be inside a shell that just called the AI CLI). Always-inject-path is honest and predictable; if the user pastes an image into a non-image-aware context, they get a usable file path on a line — a better failure mode than silent drop.
- **Persistent paste history directory** (e.g. `~/.local/share/lmux/pastes/`). Rejected: indefinite disk growth for what should be ephemeral. If the user wants to keep an image, they can `cp` it; the path on the line tells them where.
- **Use the system temp dir (`/tmp`) directly without `XDG_RUNTIME_DIR`.** Rejected on multi-user systems: `/tmp` is world-writable; pasted images can be sensitive. `XDG_RUNTIME_DIR` is mode-0700 per-user by spec.
- **Block paste when the clipboard holds an image and there's no sensible target.** Rejected: cockpit doesn't know what's "sensible"; the user does.

## Consequences

- **+** `claude paste-the-image-here` works immediately: clipboard image → PNG path on the prompt → user hits Enter → CLI reads the file.
- **+** Falls through transparently to text paste; no behavioral regression for users who paste mostly text.
- **+** No persistent disk footprint. Tmpfs handles cleanup; cockpit holds zero state about pasted images.
- **−** A path on the line is not the same as "the image" — a user who expected inline image rendering will be surprised. Documentation needs to call this out.
- **−** PNG re-encoding cost on every image paste. Acceptable: paste is interactive (single-image, user-paced); not a hot path.
- **−** PNG is the only output format. Pasting a JPEG re-encodes losslessly to PNG (size grows). Acceptable for v0.2; if anyone complains, add a "preserve format if known" branch.
- **−** No clipboard-watch / no auto-paste; the user still has to invoke paste explicitly. Intentional — autopaste is a footgun.

## Follow-up

- Configurable max image size; reject pastes above the limit with a status-bar message instead of writing a 50 MB PNG.
- Configurable paste dir (some users will want pastes outside tmpfs so they survive a logout).
- Consider whether to inject `file://<path>` instead of bare path for environments that prefer URIs.
- If the v0.2 status bar lands first, surface "📎 image pasted (<filename>)" briefly so the user knows what got injected.
