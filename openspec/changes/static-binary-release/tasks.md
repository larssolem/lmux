## 1. Build metadata pipeline

- [ ] 1.1 Add `crates/lmux/build.rs` emitting `LMUX_VERSION`, `LMUX_GIT_SHA`, `LMUX_BUILD_TIME`
- [ ] 1.2 Expose the three as compile-time constants via `env!(...)` in `crates/lmux/src/version.rs`
- [ ] 1.3 `lmux --version` prints the three; `--version --json` emits a structured payload
- [ ] 1.4 Unit tests pin the format

## 2. Static build pipeline

- [ ] 2.1 Pin the Zig toolchain version in `rust-toolchain.toml` (or adjacent `toolchain.toml`)
- [ ] 2.2 Dockerfile `packaging/build.Dockerfile` based on `manylinux_2_31` with the pinned Zig + Rust
- [ ] 2.3 `make release-binary` target that runs the Dockerfile locally; output is a stripped LTO binary
- [ ] 2.4 Verify symbol strip reduces size ≥ 20% vs unstripped
- [ ] 2.5 Document reproduction steps in `BUILD.md`

## 3. Binary-size budget

- [ ] 3.1 CI step computes the stripped size and fails if > 80 MiB
- [ ] 3.2 Warn (don't fail) if > 70 MiB
- [ ] 3.3 Size is recorded in the release notes automatically

## 4. GitHub Actions release workflow

- [ ] 4.1 `.github/workflows/release.yml` triggered on `v*` tag push and manual `dry_run` dispatch
- [ ] 4.2 Build step invokes the Dockerfile and produces `lmux-<version>-linux-x86_64.tar.zst`
- [ ] 4.3 SHA256 step produces `<artifact>.sha256` and writes the hash to a file
- [ ] 4.4 Cosign step signs the tarball with keyless OIDC, producing `<artifact>.sig` + `<artifact>.cert`
- [ ] 4.5 Release-create step uploads all four files to the GitHub Release; release notes reference the sigstore verification recipe

## 5. Self-verify subcommand

- [ ] 5.1 Release workflow computes SHA256 of the final binary and rebuilds once more with `--cfg 'lmux_sha="<hash>"'` so the constant is embedded
- [ ] 5.2 `lmux self-verify` reads `/proc/self/exe`, computes SHA256, compares against the compile-time constant; exit code `0` on match, `1` on mismatch
- [ ] 5.3 On mismatch, print a short diagnostic (expected vs actual) and suggest re-downloading
- [ ] 5.4 Unit test with a fake constant verifies the comparator

## 6. Cross-distro smoke test

- [ ] 6.1 After artifact build, three parallel jobs: `ubuntu:22.04`, `fedora:40`, `archlinux:latest`
- [ ] 6.2 Each job downloads the artifact (or consumes it via CI artifact sharing), extracts, runs `./lmux --version`, `./lmux help`, `./lmux session list`
- [ ] 6.3 Any job failing marks the release as failed and prevents publish
- [ ] 6.4 Smoke-test logs are attached to the release workflow summary

## 7. Verification documentation

- [ ] 7.1 `packaging/verifying.md` with a five-step cosign + SHA256 recipe
- [ ] 7.2 README "Install" section points at the verifying doc
- [ ] 7.3 Include example successful + failed verification output

## 8. Ancillary artifacts

- [ ] 8.1 Emit a `cargo binstall`-compatible metadata block in `crates/lmux/Cargo.toml` (`[package.metadata.binstall]` with a URL template pointing at the release tarball pattern)
- [ ] 8.2 Test `cargo binstall lmux` round-trip from the release

## 9. ADR bookkeeping

- [ ] 9.1 Update ADR-0013 status to `Shipped` with the `v0.3.0` reference
- [ ] 9.2 Link from `packaging` spec to ADR-0013

## 10. Release dry-run

- [ ] 10.1 Tag `v0.3.0-rc1`, trigger `dry_run: true` → artifact produced, smoke tests pass, nothing published
- [ ] 10.2 Resolve any findings
- [ ] 10.3 Tag `v0.3.0` → full release published
