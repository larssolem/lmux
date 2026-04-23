# ADR-0010: Sandbox shared-vs-isolated defaults

- Status: **Accepted** (bind table adopted as v0.3 first-level default; Bubblewrap spike downgraded to validation-during-dev)
- Date: 2026-04-21 (proposed) · 2026-04-21 (accepted — single-tier bubblewrap default)
- Deciders: Lars
- Blocks: v0.3

## Context

The `isolated` template (ADR-0009, ADR-0011's alias `build-safe`) is the single sandbox template shipping in v0.3. Its defaults — what the sandboxed pane *sees* of the host filesystem, network, and secrets — are the whole UX battle. Web research surfaced the concrete failure mode: shared Postgres sockets, shared `node_modules` caches, shared auth tokens *want* to flow through; if we isolate them too aggressively, the sandbox is "technically correct, practically unusable" and the user disables it. If we isolate them too little, it's theatre.

Originally this decision was gated on a Bubblewrap spike. On 2026-04-21 the decision was taken to **adopt the proposed bind table as the v0.3 first-level default** without the spike as a blocker — rationale: the bind set below is derivable from well-understood Bubblewrap semantics, the alternatives ("share everything" / "isolate everything") are already known failure modes, and v0.3 ships one template (`isolated`) anyway. The spike is downgraded from a gate to a validation exercise during v0.3 development — if it surfaces real pain (overlay perf, bind-mount latency on Btrfs/ZFS), the bind table is revised *then*, not pre-emptively.

"Bubblewrap as first level" means: one well-chosen default template in v0.3; opt-in overrides via TOML options; richer sub-templates (ADR-0010 alternative #3) are an explicit v0.4 concern, not v0.3.

## Decision

`isolated` template default bind set:

| Host resource | Default in sandbox | Rationale |
|---------------|--------------------|-----------|
| `$HOME` (top-level) | read-only bind | Dotfiles and SSH pubkeys are often incidentally needed; writes stay in overlay |
| `$HOME/.cache/{npm,pip,cargo,uv}` | read-only bind | Shared caches are the #1 pain; isolating them kills dev loops |
| `$HOME/.ssh` | **not bound** | Private keys should not leak into sandboxed agents by default |
| `$HOME/.config/*/auth*`, `$HOME/.netrc`, `$HOME/.aws`, `$HOME/.gnupg` | **not bound** | Auth tokens are exactly the leak vector to prevent |
| `$PWD` (current project) | overlay (read-write, upper layer per-pane) | Lets the pane edit code; changes don't cross to host until explicit |
| `/tmp` | fresh tmpfs | Isolate ephemeral state; prevent cross-agent leakage |
| `/run/user/$UID/lmux/*.sock` | bind (so the smart-open bus works) | Required for bus (ADR-0008) |
| Unix sockets in `/tmp/*.sock`, `/var/run/postgresql/` | **opt-in bind** via template option | Allows shared Postgres when the template says so |
| Network | `--unshare-net` (deny-by-default); palette-granted allowlist | Matches stated brief policy |
| `/etc/{passwd,group,hosts,resolv.conf}` | read-only bind | Needed for any non-trivial tool to work |
| `/dev/{null,zero,urandom,tty}` | bwrap defaults | Standard |

Anti-goals explicitly encoded:

- Default sandbox must not need a 5-step README to be productive (anti-metric from brief).
- Default sandbox must not permit outbound network without an explicit palette action.
- Default sandbox must not expose `~/.ssh` / auth tokens.

## Alternatives considered

- **"Share everything, isolate only `$PWD`."** Rejected as default: becomes security theatre; defeats the token-leak mitigation that's half the value prop.
- **"Isolate everything, bind only explicitly."** Rejected as default: every agent run would need a setup dance; kills adoption.
- **Named sub-templates inside `isolated`** (e.g. `isolated-node`, `isolated-python`, `isolated-postgres`). Rejected for v0.3: multi-template scope is explicitly v0.4. One well-chosen default, then opt-in overrides.

## Consequences

- **+** Dev loops that depend on shared caches keep working.
- **+** Secrets and outbound network are walled by default.
- **+** The `networked` / opt-in socket bind / shared-Postgres flags become configurable *escapes*, not defaults.
- **−** If the Bubblewrap spike finds that overlay + bind-mount performance on Btrfs / ZFS hurts dev loops, we revise down to minimal binds and take the adoption hit. Spike must measure.
- **−** `~/.config/*/auth*` glob is heuristic; real apps hide secrets in creative places. Mitigation: document the "things `isolated` hides" list and let users add paths to the hide-list.

## Follow-up

- **During v0.3 dev:** exercise the bind table against real workflows (node build, Python uv project, Rust cargo build, shared Postgres socket). Record concrete pain-points; revise the bind table in-place rather than re-opening this ADR for minor tuning.
- **Revisit triggers (would re-open this ADR):**
  - Overlay performance on Btrfs/ZFS makes `$PWD` editing noticeably laggy → consider falling back to read-write bind with undo via git.
  - `~/.config/*/auth*` heuristic proves insufficient (tokens leak through paths we didn't anticipate) → switch to explicit allowlist model.
  - v0.3 dogfood reveals `/tmp` tmpfs breaks a common tool → promote to opt-in bind with a documented escape hatch.
- **v0.4 scope:** named sub-templates (`isolated-node`, `isolated-python`, etc.) re-open the "one template vs many" question. Not part of this ADR.
- **ADR-0009 kill criterion:** still active — if bubblewrap itself proves unusable (e.g. userns disabled on target distros), sandbox scope for v0.3 must shrink independently of this bind table.
