# ADR-0001: Rendering stack — Rust + libghostty + GTK4 + portable-pty

- Status: Accepted
- Date: 2026-04-21
- Deciders: Lars
- Blocks: v0.1

## Context

lmux needs a terminal rendering stack that: (a) reproduces Ghostty's VT fidelity without writing a VT engine from scratch, (b) links statically into a single Rust binary, (c) supports GUI chrome around panes (sidebar, tabs, docked satellites), (d) drives PTYs with resize + keyboard correctly across shells.

Ghostty's `libghostty-vt` is stable and zero-deps; the embed API is pre-1.0 but functional. cmux proves the embedding works on macOS via Apple-only parts of the API; Linux builds only expose `libghostty-vt`.

## Decision

The rendering stack is:

- **Rust** as host language.
- **libghostty-vt** (Zig, statically linked as `ghostty-vt-static`) as the VT core, consumed via a bindgen-generated FFI layer from `ghostty/vt.h`.
- **GTK4 0.9 + pangocairo 0.20** as the surface for drawing grid cells and hosting GUI chrome.
- **portable-pty 0.9** + `async-channel 2` for PTY management and reader→UI plumbing.
- Vendor Ghostty as a Zig subproject; `cargo build.rs` runs `zig build --release=fast` to produce the static lib.
- Bindgen allowlist: `ghostty_*`, `Ghostty*`, `GHOSTTY_*`; flag `-DGHOSTTY_STATIC`.

Validated end-to-end in `spikes/libghostty-ffi/` — FFI, PTY, keyboard, and resize all work.

## Alternatives considered

- **Write a VT engine in Rust (vte-rs, alacritty_terminal).** Rejected: reinvents Ghostty's shape handling and edge cases; violates Display-Don't-Duplicate.
- **Embed Ghostty via its full `ghostty.h` API.** Rejected: Apple-only on Linux builds; not portable.
- **GTK3 instead of GTK4.** Rejected: GTK4 is the long-term target for KDE + wlroots tooling and has Wayland-native rendering.
- **winit + wgpu direct rendering.** Rejected: higher custom-chrome cost; GTK4 gives us sidebar/tabs/menus for free.
- **Iced / Slint / egui.** Rejected: weaker Wayland + accessibility story than GTK4 for the cockpit chrome we need.

## Consequences

- **+** Single static binary is achievable — a named trust signal for the target audience.
- **+** Ghostty-native fidelity without a VT rewrite; credibility with the Ghostty community.
- **−** Zig is required at build time; adds a toolchain step beyond plain Cargo. Mitigation: pin Zig version in `build.rs`, document in `BUILD.md`.
- **−** libghostty embed API is pre-1.0; breaking changes probable. Mitigation: kill criterion in brief — "incompatible API break ≥2× in 3 months" triggers a pivot review.
- **−** GTK4 on Wayland imposes accessibility + IME + HiDPI behaviour on us that libghostty doesn't control. Mitigation: treat GTK quirks as known quantity; test against KWin + nested wlroots.

## Follow-up

- Track libghostty upstream for API stabilisation signals before v0.3.
- Evaluate `ghostty-vt` Kitty-graphics + OSC-clipboard additions when they ship.
