# ADR-0013: Distribution channel — static binary authoritative

- Status: Accepted
- Date: 2026-04-21
- Deciders: Lars
- Blocks: v0.3 release

## Context

lmux's rendering stack (ADR-0001) is explicitly chosen so that a single static Rust+Zig-built binary is achievable. The target audience (Linux power users burned by Electron, Python dependency hell, and Flatpak sandbox quirks) treats single-binary delivery as a trust signal.

Linux distribution has three broad channels: (a) direct static binary via GitHub Releases, (b) Flatpak on Flathub, (c) native distro packages (AUR for Arch-likes, Nixpkgs, etc.). Each has operational cost, especially for a solo dev.

## Decision

The **static binary** is the authoritative release artifact. Flatpak and AUR are secondary, community-maintained channels if and when volunteers want to maintain them.

Concretely:

- `v0.3` ships as a `lmux-<version>-linux-x86_64` tarball from GitHub Releases, statically linked (Zig-built libghostty, MUSL libc if feasible, otherwise glibc-pinned to a wide-compat baseline).
- README documents verifying the binary (SHA256 + sigstore once set up).
- **No** official Flatpak, AUR package, or Nix derivation is committed to by the project. If a contributor submits or maintains one, lmux links to it in README with a clear "community-maintained" label.
- Auto-updater is **not** shipped. Users update via `cargo binstall`-style tooling or manual download.

## Alternatives considered

- **Flatpak as primary.** Rejected: Flatpak's sandbox interacts badly with lmux's own sandbox (nested bwrap); `ext-foreign-toplevel-list-v1` and `org.kde.KWin.Scripting` access from inside Flatpak sandbox is fragile.
- **AUR as primary.** Rejected: Arch-only; excludes Fedora/Debian/Ubuntu users.
- **Distro packaging as primary.** Rejected: requires ongoing maintenance relationships with each packager; solo-dev over-reach.
- **Snap.** Rejected: ecosystem fit; Ubuntu-centric; cultural mismatch with the enthusiast-Linux audience.
- **AppImage.** Rejected: the container format's value proposition (portability across distros) overlaps with a static binary, without the transparency; static binary wins.

## Consequences

- **+** Zero packaging overhead for the solo maintainer.
- **+** Trust signal reinforced — matches "single static binary, no runtime, no daemon, no Electron" pitch.
- **+** Fewer moving parts when debugging issues: everyone runs the same binary.
- **+** Contributors who want to maintain a Flatpak or AUR package are welcomed without commitment.
- **−** Users accustomed to `apt install` / `pacman -S` will either wait for a community package or feel friction. Acceptable for v0.3.
- **−** No auto-update: updates are manual. Acceptable: this audience runs `rustup update` monthly without complaint.
- **−** Static linking increases binary size. Mitigation: strip symbols; LTO; compare with Ghostty's own binary size as a sanity check.

## Follow-up

- Set up signed release process before v0.3 (sigstore or equivalent).
- Monitor for community packaging PRs post-v0.3; link from README.
