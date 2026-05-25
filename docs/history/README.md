# History

Design-era artifacts preserved from the lmux bootstrap phase. These documents captured the project's *why*, *what*, and *how-we-got-here* before the switch to OpenSpec (2026-04-23) made `openspec/specs/` and `openspec/changes/` the living source of truth.

They are **read-only reference**. For current requirements, consult `openspec/specs/<capability>/spec.md`. For current proposals, `openspec/changes/<name>/`.

## What's here

### Idea lineage

| File | Contents |
|---|---|
| [brainstorming-2026-04-20.md](brainstorming-2026-04-20.md) | Original 60-idea brainstorm. Four techniques (What If / Mind Mapping / SCAMPER / Resource Constraints). Seed of every design principle that followed. |
| [product-brief.md](product-brief.md) | The project's positioning, target audience, and the six design principles that override features ("Display, Don't Duplicate"; "Editor-Agnostic > Vendor Deep Dive"; "Cohesive Workspace Feel Is the Success Bar"; …). Start here if you want the project's soul in one page. |
| [product-brief-distillate.md](product-brief-distillate.md) | Dense-form companion to the brief, meant for cheap context injection. |

### v0.2 specification (still referenced by FR numbers)

| File | Contents |
|---|---|
| [prd-v0.2.md](prd-v0.2.md) | v0.2 PRD with FR1–FR63 / NFR1–NFR33. Several ADRs cite these FR numbers; keep this file intact unless the ADRs are updated to reference openspec requirements instead. |

### Implementation history

| File | Contents |
|---|---|
| [v0.2-progress.md](v0.2-progress.md) | Point-in-time progress log kept across v0.2 coding sessions. Canonical answer to "what actually shipped, when." |
| [spike-compositor-ipc.md](spike-compositor-ipc.md) | Spike spec that informed ADR-0005 (KWin MVP) and ADR-0006 (Hyprland-first wlroots). Cited from ADR-0006. |
| [spike-kwin-lifecycle.md](spike-kwin-lifecycle.md) | KWin script lifecycle probe. Fed ADR-0011. |
| [e2e-test-strategy.md](e2e-test-strategy.md) | Proposed test pyramid + seams (control socket, layout state file, tracing). Not yet captured as an openspec capability; revisit when the e2e harness lands. |

## What was dropped

- Epics files (11-epic decomposition for v0.2) — fully superseded by `openspec/specs/` + `openspec/changes/`.
- Architecture docs (v0.1 + v0.2) — superseded by ADRs under `docs/adr/` plus the `## Purpose` sections of each capability spec.
- Prior-version drafts of PRD/epics/brief that were superseded before v0.2 shipped.
- Planning-tool framework config trees — removed because they were not lmux artifacts.
