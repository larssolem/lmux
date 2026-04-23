# ADR-0014: Product name

- Status: **Deferred** until a named trigger
- Date: 2026-04-21
- Deciders: Lars
- Blocks: public v0.3 pitch

## Context

`lmux` is the working handle used throughout the brainstorm, spike work, and this brief. It stands for "Linux mux" — an obvious clone-of-cmux origin that the product has since outgrown: lmux is no longer a cmux port but a distinct anchor/satellite dev cockpit.

Keeping `lmux` through v0.1/v0.2 is harmless — the audience is the author. But the name signals "terminal multiplexer" (via the `mux` suffix) and "Linux port of cmux" (via the initial L), both of which are now misleading. A final name decision has real weight only when the project is pitched publicly.

## Decision (deferred)

Defer until **the public v0.3 pitch is being prepared**. Until then, `lmux` remains the handle in code, docs, and ADRs.

**Explicit trigger to re-open this ADR:**

- Author is within ~2 weeks of publishing a v0.3-facing landing page, README pitch, or HN/Lobsters post.
- *Or* a trademark / name conflict surfaces (e.g. a project named `lmux` already exists — quick search: none notable as of 2026-04-21).
- *Or* the author receives direct feedback from ≥3 unrelated people that the current name is actively confusing.

## Constraints on the final name (when we decide)

- Short (≤6 characters preferred); pronounceable aloud.
- Not already a trademark or well-known OSS project.
- Doesn't imply "terminal multiplexer" (the product is more than that).
- Doesn't imply "Linux cmux clone" (the product has diverged).
- Leaves room for non-Linux future (even if not planned) — avoid "linux-" prefixes.
- Ideally gestures at the anchor/satellite / cockpit / workspace-tenant shape.

Candidate seeding (unvetted brainstorm; none of these are decisions):

- `cockpit` — too generic, conflicts with cockpit-project.org.
- `dock` / `dockyard` — dock metaphor fits satellite docking.
- `anchor` — literal but generic.
- `tenant` — encodes "workspace tenant, not WM" explicitly.
- `helm` / `bridge` / `nav` — nautical / cockpit.

## Alternatives considered (for the meta-decision)

- **Rename now.** Rejected: the author is the only user through v0.2; any effort spent on naming is effort not spent on shipping.
- **Commit to `lmux` forever.** Rejected: the name actively misdescribes the product; bad public-launch baggage.
- **Rename at v0.2 dogfood milestone.** Rejected: v0.2 is still author-only; public naming costs are paid at v0.3.

## Consequences

- **+** Zero naming effort until it matters.
- **+** Public naming benefits from ~4 months of living with the product shape — better instincts on what the name should signal.
- **−** Code, ADRs, CLI binaries, socket paths (`lmux.sock`), config dirs (`~/.config/lmux/`) all have to be renamed at the trigger. Mitigation: grep-replaceable, one day of work; plan a migration commit for the rename.
- **−** Any external mentions (spike commit messages, this ADR file, memory files) remain `lmux`-branded forever. Acceptable: these are audit history, not product identity.

## Follow-up

- At the trigger: re-open this ADR with status → Accepted, list shortlisted names, pick one, file a rename commit.
