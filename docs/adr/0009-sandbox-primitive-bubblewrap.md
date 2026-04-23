# ADR-0009: Bubblewrap as v0.3 sandbox primitive

- Status: Accepted
- Date: 2026-04-21
- Deciders: Lars
- Blocks: v0.3

## Context

Per-pane sandboxing is lmux's flagship v0.3 differentiator — not just "isolation exists," but isolation that composes with the tooling AI agents already use. The web research confirmed Bubblewrap (`bwrap`) is the de-facto Linux consensus: Claude Code defaults to it; it ships packaged on every major distro; it's unprivileged and based on user namespaces. Other options (Podman, LXC, Firecracker/crosvm) solve different problems at higher operational cost.

lmux's original design envisioned a **tiered pluggable backend** from None → Bubblewrap → LXC/Podman/Docker → microVM. Shipping all tiers in v0.3 is unrealistic given solo-dev scope; the question is which tier ships first.

## Decision

**Bubblewrap only for v0.3.** Backends are pluggable in the architecture (`trait SandboxBackend`), but v0.3 ships exactly one implementation. Specifically:

- `isolated` template (alias: `build-safe`) is the only template shipped in v0.3 (see ADR-0010).
- Network allowlist is palette-granted, implemented via `bwrap --unshare-net` + opt-in socket forwarding or slirp-style shim (to be validated by Bubblewrap spike).
- Overlay filesystem uses `overlay`-fs + `bwrap`'s bind-mount semantics. `lmux commit` / `lmux revert` is **deferred to v0.4** (scope cut).
- Future backends (Podman, Firecracker) are v0.4+; the trait shape is reserved but not exercised.

## Alternatives considered

- **Podman as primary v0.3.** Rejected: heavier runtime; daemon/rootless edge cases; overkill for "sandbox a pane"; ecosystem expectation is `bwrap` for this class of task.
- **Firecracker / crosvm microVM.** Rejected: great for `untrusted-full` template later, but v0.3 is too early; kernel and image management scope-creeps badly.
- **Systemd-run / Landlock only.** Rejected: Landlock is used by OpenAI Codex, but its policy surface is narrower than `bwrap`'s and less well-documented for non-developers. Good future companion, not a primary.
- **No sandbox in v0.3 (defer to v0.4).** Rejected: removes the flagship differentiator; lmux without sandbox is "another multiplexer."
- **Ship all tiers in v0.3.** Rejected: solo-dev scope violation; pick one, ship it well.

## Consequences

- **+** Composes with tools (Claude Code, Aider) that already understand `bwrap`.
- **+** Zero-daemon, unprivileged, packaged: minimal operational ask from the user.
- **+** Forces the UX conversation (ADR-0010 shared-vs-isolated defaults) to be the v0.3 focus, not runtime engineering.
- **−** Bubblewrap is Linux-only. Acceptable: lmux is Linux-only.
- **−** Bubblewrap cannot sandbox everything cleanly — GUI processes needing X11 sockets, Wayland sockets, or DBus are fiddly. Mitigation: the v0.3 `isolated` template targets CLI/build workloads; GUI-in-sandbox is v0.4+.
- **−** Resource broker (auto-allocated ports, per-workspace Postgres schema) and commit/revert are attractive features we're *not* shipping in v0.3. Documented in v0.4 backlog.

## Follow-up

- Bubblewrap spike (pending) must validate network toggle, overlay mounts, and the default bind set (see ADR-0010).
