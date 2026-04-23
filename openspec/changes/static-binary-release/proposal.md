## Why

ADR-0013 picked the **static binary** as the authoritative v0.3 distribution artifact: a tarballed, statically-linked `lmux` on GitHub Releases, verifiable via SHA256 + sigstore. v0.2 has shipped none of that — there's no release workflow, no signing story, no tested cross-distro boot, no documented verification path. That's an explicit v0.3 gate because the whole "single static binary, no daemon, no runtime" pitch is a trust-signal that only lands if the artifact exists and is reproducible.

This change is pure release engineering. It introduces a new `packaging` capability (no runtime surface) whose requirements describe the build, sign, publish, and verify surface.

## What Changes

- A GitHub Actions workflow `.github/workflows/release.yml` that, on `v*` tag, builds the static binary (libghostty via Zig → linked statically; glibc-pinned baseline; LTO + strip), packages it as `lmux-<version>-linux-x86_64.tar.zst`, computes SHA256, signs with sigstore cosign, uploads all three to the GitHub Release.
- A `lmux --version` that prints the exact build metadata (version, git sha, build time) embedded at compile time via `build.rs`.
- A `lmux self-verify` subcommand that checks the running binary's SHA256 matches its expected value from an embedded-at-build-time manifest (optional self-integrity check).
- Documented verification recipe in README: `cosign verify-blob --certificate ... lmux.tar.zst` + SHA256 cross-check.
- Bin-size budget: CI fails if the stripped binary exceeds 80 MiB (pick a number that leaves headroom; Ghostty is ~40 MiB for comparison).
- A `BUILD.md` section on "Reproducing a release build locally" with the exact toolchain versions.

Out of scope for this change: AUR, Flatpak, Nix derivations; auto-update infrastructure; in-product update prompts. These may become community-maintained channels later (ADR-0013).

## Capabilities

### New Capabilities

- `packaging`: the full release + distribution surface, with requirements for reproducible builds, signing, SHA verification, version metadata, and binary-size budget.

### Modified Capabilities

(none)

## Impact

- Code: new `build.rs` in `crates/lmux` that emits `LMUX_VERSION`, `LMUX_GIT_SHA`, `LMUX_BUILD_TIME`, `LMUX_BINARY_SHA256` env-vars used by the runtime. New `lmux self-verify` subcommand thin wrapper. No runtime-behaviour changes.
- CI: new release workflow; adds a dependency on sigstore's GitHub Actions integration. Cosign-signed artifacts use the GitHub OIDC identity (no long-lived keys).
- Docs: new `packaging/verifying.md` + README section. ADR-0013 marked `Shipped` once this change lands.
- Tests: a CI smoke test that starts the built binary in a clean Ubuntu 22.04 container, runs `lmux --version`, `lmux session list`, and exits; same smoke test against Fedora 40 and Arch latest.
- No end-user runtime changes except the new subcommand.
