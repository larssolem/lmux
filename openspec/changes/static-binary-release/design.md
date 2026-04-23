## Context

The lmux rendering stack (ADR-0001) was picked so a single static binary is *achievable*. The packaging story has to make it *actual*. That means a build pipeline reproducible by a third party, an artifact signed with keys the community can verify without trusting the maintainer's laptop, and documentation that a security-conscious user can follow in five minutes.

GitHub Actions + sigstore cosign (keyless signing via OIDC) is the industry-standard answer for open-source projects that don't want to manage long-lived signing keys. Artifact verification is a two-step: (1) `cosign verify-blob` using the published certificate; (2) SHA256 cross-check against the release notes.

## Goals / Non-Goals

**Goals**
- A repeatable build: anybody with `cargo` + `zig` at the pinned versions can locally produce the same binary (give or take non-determinism in the linker, documented).
- Keyless signing via sigstore/cosign.
- A concise verification recipe a skeptical user can run in the terminal.
- Binary-size discipline: fail fast if a dependency change blows the budget.
- Multi-distro smoke test so release artifacts are actually tested outside the maintainer's Arch box.

**Non-goals**
- Reproducible down to the byte. Linker-induced non-determinism is allowed; we pin toolchains instead.
- Multiple architectures. x86_64 Linux only for v0.3. ARM64 is a follow-up change.
- Package manager integration (AUR, Flatpak, Nix, Snap, AppImage). Explicitly not us.
- Auto-update infrastructure.
- Code-signing for other OSes. Linux-only per ADR-0001.

## Decisions

### D1 — MUSL vs glibc: glibc with a pinned baseline

Decision: build against glibc 2.31 (Ubuntu 20.04 LTS baseline), via a `manylinux`-style builder container. Rationale:

- MUSL is cleaner but libghostty's Zig build-path is known to have MUSL friction; glibc avoids that tax.
- 2.31 is old enough that every mainstream distro running v0.3 has a newer libc.
- `manylinux_2_31` is a maintained container recipe we can lift.

Alternative considered: MUSL via `alpine:latest`. Rejected for ghostty linkage issues; revisit for v0.4.

### D2 — sigstore cosign keyless signing

Decision: sign every release artifact with cosign's keyless mode (OIDC from GitHub). Rationale: no key material to lose; verification is `cosign verify-blob --certificate-identity-regexp '^https://github.com/.*/lmux/.*$' --certificate-oidc-issuer 'https://token.actions.githubusercontent.com' ...`; users don't need GPG.

Alternatives considered: GPG with maintainer key (keymat loss / rotation pain); no signing (insufficient trust signal for the pitch).

### D3 — One artifact: a single `.tar.zst`

Decision: the release is exactly one binary artifact (`lmux-<version>-linux-x86_64.tar.zst`) plus the signature (`.sig`) and certificate (`.cert`) plus a SHA256 file. No per-component split; no separate docs tarball. Rationale: minimizes download friction; signatures cover exactly what the user runs.

### D4 — Version + integrity embedded via `build.rs`

Decision: `build.rs` reads `CARGO_PKG_VERSION`, `git rev-parse HEAD`, and the output-file-will-be-hashed placeholder, then at the end of the final build, the release workflow computes the SHA256 of the produced binary and re-injects it via a `--cfg lmux_sha=<hash>` trick. The `lmux self-verify` subcommand reads that constant and compares against `sha256sum(/proc/self/exe)`. A mismatch prints a loud warning but continues (it's a check, not a gate).

Alternatives considered: pure runtime SHA of `/proc/self/exe` without a compiled-in expected value (can't tell the user what the binary "should be" if tampered); detached checksum file in the tarball (can be tampered in the same motion). Compile-in + runtime-check strikes the right balance.

### D5 — Binary-size budget: 80 MiB stripped

Decision: CI fails the release workflow if the final stripped binary exceeds 80 MiB. Rationale: Ghostty's own stripped binary is ~40 MiB; we add GTK, tokio, gtk4-rs, kbdsrc etc. and should stay under double. The budget is declarative in `.github/workflows/release.yml` so any PR can see the current headroom.

Revisit trigger: ARM64 support (D3 becomes two artifacts), major dependency swaps.

### D6 — Cross-distro smoke test matrix

Decision: the release workflow runs three parallel container jobs after producing the tarball: `ubuntu:22.04`, `fedora:40`, `archlinux:latest`. Each job `tar xf`'s the artifact, runs `./lmux --version` and `./lmux help`, and `./lmux session list` (expected to fail with "no cockpit running" — that's still a reachable-binary signal). Any job failing fails the release.

### D7 — `lmux self-verify` is informational, not enforcing

Decision: `self-verify` prints `match` / `mismatch` and exits `0`/`1` respectively; it never modifies state. The cockpit startup does not call it by default (would slow boot for a check that almost never fires). Users concerned about supply chain can alias it to a post-install check or run it from a cron.

### D8 — Release tagging convention

Decision: tags are `v<semver>` (e.g. `v0.3.0`). The workflow triggers on `v*` tags; manual-dispatch with `dry_run: true` builds the artifact without publishing, for testing the pipeline.

## Risks / Trade-offs

- *Risk: the glibc 2.31 baseline doesn't cover every LTS in the wild.* → Mitigation: the cross-distro smoke test catches regressions; readme documents the baseline.
- *Risk: sigstore's OIDC flow changes or an outage blocks a release.* → Mitigation: the workflow supports a manual-key fallback (documented); we fall back on that only if sigstore is down for > 6 hours on a release day.
- *Risk: binary size balloons from a transitive dep.* → Mitigation: the 80 MiB budget (D5) fails the release; a PR-level warning triggers above 70 MiB.
- *Risk: Zig toolchain regressions affect libghostty linkage.* → Mitigation: the Zig version is pinned in `rust-toolchain.toml`-style metadata; the BUILD.md reproducibility recipe calls it out explicitly.

## Migration Plan

- Tag `v0.3.0` triggers the workflow end-to-end for the first time; `dry_run: true` runs precede it.
- Existing README "install" section is replaced with a verifying-the-download recipe.
- ADR-0013 marked `Shipped` in the same PR that lands the workflow.

## Open Questions

- Do we want a `cargo binstall` manifest alongside the tarball? Leaning yes — it's free for us once the sig+sha exist — tasks.md includes it.
- Should `lmux self-verify` be called on every cockpit start behind a config flag? Leaning no for performance reasons; defer until a concrete threat model asks for it.
- Do we publish the workflow's build logs with the release? Leaning yes — supply-chain transparency costs us nothing.
