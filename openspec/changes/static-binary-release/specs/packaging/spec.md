## ADDED Requirements

### Requirement: Single static binary artifact

The lmux release SHALL ship as exactly one compressed tarball `lmux-<version>-linux-x86_64.tar.zst` containing a statically-linked `lmux` executable, built from a documented and reproducible toolchain, with a stripped binary size under 80 MiB and a glibc baseline no newer than 2.31.

#### Scenario: Release artifact naming

- **WHEN** the release workflow produces an artifact for tag `v0.3.0`
- **THEN** the artifact file name matches `lmux-0.3.0-linux-x86_64.tar.zst` and the tarball root contains exactly one file named `lmux`

#### Scenario: Binary-size budget enforced

- **WHEN** the stripped binary produced by the release workflow exceeds 80 MiB
- **THEN** the workflow fails before the signing step; no release is published

#### Scenario: glibc baseline preserved

- **WHEN** the binary is copied to a clean `ubuntu:20.04` (glibc 2.31) container and executed
- **THEN** `./lmux --version` prints successfully; no `GLIBC_...` symbol-version errors are reported

#### Scenario: Reproducible build recipe exists

- **WHEN** a third party follows `BUILD.md`'s "Reproducing a release build" section on a supported host
- **THEN** they produce a binary whose SHA256 matches the published artifact modulo documented linker non-determinism (covered by the recipe)

### Requirement: Signed release with sigstore cosign

Every release artifact SHALL be signed via sigstore cosign's keyless OIDC flow; the release MUST publish the detached signature, the signing certificate, and a SHA256 file alongside the tarball; the README MUST document a self-contained verification recipe.

#### Scenario: Every release has a signature alongside the tarball

- **WHEN** a release is published at tag `v<semver>`
- **THEN** the GitHub Release page contains `lmux-<version>-linux-x86_64.tar.zst`, `<artifact>.sig`, `<artifact>.cert`, and `<artifact>.sha256`; all four artifacts are produced by the same workflow run

#### Scenario: cosign verify-blob succeeds

- **WHEN** a user runs `cosign verify-blob --certificate <artifact>.cert --signature <artifact>.sig --certificate-identity-regexp '^https://github.com/.*/lmux/.*$' --certificate-oidc-issuer 'https://token.actions.githubusercontent.com' <artifact>`
- **THEN** cosign reports successful verification; the certificate identity matches the lmux GitHub repository

#### Scenario: Verification recipe documented

- **WHEN** a user follows `packaging/verifying.md`
- **THEN** the recipe walks from "download" to "cosign verified + SHA256 matched" in five or fewer commands without external references beyond cosign's documented flags

### Requirement: Compile-time version and integrity metadata

The `lmux` binary SHALL embed its version, git SHA, build timestamp, and a self-SHA256 at compile time; the `lmux --version` subcommand SHALL print the metadata and the `lmux self-verify` subcommand SHALL compare the running binary's SHA256 against the embedded value.

#### Scenario: `lmux --version` prints structured metadata

- **WHEN** the user runs `lmux --version`
- **THEN** stdout includes `version=<semver>`, `git_sha=<hex>`, `built_at=<iso8601>`; `--json` emits a machine-readable object with those three fields

#### Scenario: `lmux self-verify` matches on untampered binary

- **WHEN** the user runs `lmux self-verify` on an unmodified release binary
- **THEN** the command exits `0` with output `match` and prints the verified SHA256

#### Scenario: `lmux self-verify` detects tampering

- **WHEN** the binary has been modified after publication
- **THEN** `self-verify` exits `1`, prints `mismatch` with both the expected and observed hashes, and suggests re-downloading from the release page

#### Scenario: Self-verify is informational, not gating

- **WHEN** `self-verify` detects a mismatch
- **THEN** no other lmux command (session CRUD, `open`, etc.) refuses to run as a consequence; the check is advisory

### Requirement: Cross-distro release smoke test

The release workflow SHALL run a smoke test of the built artifact on at least three current distro containers (Ubuntu LTS, Fedora, Arch) and MUST block publication if any smoke test fails.

#### Scenario: Smoke test invokes `--version` on each distro

- **WHEN** the release workflow executes the smoke-test matrix
- **THEN** on each of `ubuntu:22.04`, `fedora:40`, `archlinux:latest` the artifact is extracted and `./lmux --version` exits `0`

#### Scenario: Smoke test failure blocks publication

- **WHEN** any smoke-test job returns a non-zero exit code
- **THEN** the release workflow fails, no artifacts are uploaded to the GitHub Release, and the workflow summary names the failing distro + command
