# Agent Guide — Crimocracy Cockpit

**BCA policy:** advisory

**Rust agent diagnostics:** advisory. Use `scripts/rust-diagnostics.ps1`. Use bounded diagnostics only for unresolved ownership, test-strength, or macro questions; commands and verification routing: [TESTING.md](TESTING.md).

**Profiles:** Universal, Stateful Application, Deterministic System, Automated Behavior Evaluation

This is the execution card for agents changing this system. It keeps high-risk
guardrails local and routes detailed contracts to their single owners. Ownership is in
[`ARCHITECTURE.md`](ARCHITECTURE.md); scope is in [`STATUS.md`](STATUS.md);
evidence rules are in [`TESTING.md`](TESTING.md); intent is in
[`GAME_DESIGN.md`](GAME_DESIGN.md); commands are in [`README.md`](README.md).

---

<!-- workspace-contract:begin (generated from ../AGENTS.md by tools/sync_agent_context.py; edit the source, not this copy) -->
## Workspace contract

Applies to every agent in every harness. Source, rationale, and evidence: [../AGENTS.md](../AGENTS.md).

The user is a solo developer. Precedence: current request → this contract → project card → standards. This is a safety kernel, not a prescribed workflow. **Hard** rules survive lower-level advice.

### AG-1 Finish the work

Finish the authorized task. On open-ended goals, iterate from evidence until the goal is met or useful in-scope improvements are exhausted; pivot around blockers instead of stopping at them.

### AG-2 Make the calls yourself

Make routine in-scope decisions yourself. Read the project card once, identify the owner and proof route, then work. Ask only when a choice materially changes the deliverable and cannot be inferred.

### AG-3 Write only in your project (hard)

Write only in the requested project. Read-only sibling inspection is allowed when it materially helps. Do not write to the workspace root, parent, or siblings; use project-ignored output or OS temp for scratch.

### AG-4 Done means the user's copy works (hard)

Verify the result in the user's actual copy, including the release, deployment, or running app when applicable. Use the smallest complete project lane and [`REVIEW.md`](../REVIEW.md) for consequential code. Claim only checked results.

### AG-5 Look at what you made

Run or render changed visual, audio, interactive, or published output and inspect the real result against project references and [`STANDARDS_QUALITY.md`](../STANDARDS_QUALITY.md). Tests do not prove appearance.

### AG-6 Real behavior over proxies

Prefer observed product behavior over proxy success. Use tests, harnesses, validators, and metrics as evidence; fix production root causes and proxy divergence rather than weakening gates or hardcoding outcomes.

### AG-7 Answer first, then stop talking

Answer questions directly. Reports state changes, verification limits, and required user action; keep long audit material in project files instead of chat.

### AG-8 Own mistakes; trust the user's evidence

On a reported failure, re-check your work before blaming setup. Try a capability before claiming it is unavailable.

### AG-9 The outcome outranks the process

Skip ceremony and optional diagnostics when they cost more than they protect. Do not skip the proof required by AG-4 through AG-6 or any hard rule.

### AG-10 Leave nothing running

Run automation without disrupting the desktop. Stop task-owned processes, release locks, and remove scratch; never kill or replace the user's program silently.

### AG-11 Git: `main`, commit, push (hard)

Work on `main`; branches or worktrees only when asked. Commit and push verified work unless told not to. Stage task-owned paths explicitly in dirty trees and never discard or sweep in unrelated work.

### AG-12 Current docs describe the present

Update the authority for changed behavior. Keep history and session notes out of current docs and comments.

### AG-13 CI is local; GitHub Actions are banned (hard)

Use repository-owned local verification. Never create, enable, invoke, or push `.github/workflows/`.

### Optional workspace accelerator

Use native navigation first. If cold-start routing is still expensive or ambiguous, run:

```text
python ../tools/agent_context.py brief crimocracy <task ...>
```

It returns bounded authority, verification, and likely-source evidence. Tool output is evidence, not authority. Use `python ../tools/agent_context.py --help` only when a specific unresolved question justifies a deeper probe. No shared analysis command is a prerequisite to editing.
<!-- workspace-contract:end -->

## Start here

1. Preserve unrelated work. Use the source map in [ARCHITECTURE.md](ARCHITECTURE.md) to identify the owning `AppState` field and module; check the relevant supported scope in [STATUS.md](STATUS.md).
2. Read the owner's `//!` contract and focused tests in `src/<owner>/tests.rs` or the owning `*_system.rs`. Locate the existing `validate_*` / `decide_*` operation before adding a path.
3. Read only the architecture, persistence, tick, or harness sections touched by the change. [GAME_DESIGN.md](GAME_DESIGN.md) owns intent, not implemented capability.
4. Select the smallest complete lane from [TESTING.md](TESTING.md), implement through the owner, and update the single authority whose contract changed.

If a route is unclear, use the bounded diagnostic declared above or search the owning domain. Reconcile conflicting documentation and implementation; neither is permission to ignore the other.

## State and operation boundaries

- `Registry` is immutable authored content; `AppState` owns runtime records, clocks, and serialized RNG streams. `src/lib.rs` owns the module inventory; [ARCHITECTURE.md](ARCHITECTURE.md) owns dependency and state maps.
- Multi-record work uses `Draft → validate_* → Validated*::commit`. Resolve references, permissions, lifecycle, capacity, ranges, and arithmetic before mutation; commit rechecks freshness through `ensure_time_current`. Rejection preserves authoritative state and indexes. Never construct `*Record` literals or patch private records from adapters, tests, importers, or tools.
- `decide_* → apply_*` separates derivation from mutation. Single-owner direct operations are valid only when every return preserves their invariants and indexes. Use the existing owner method rather than inventing a parallel operation family.
- Maintain derived indexes, schedules, lifecycle membership, and record versions with their owner. Reserve IDs through `IdCounters::reserve` where allocation requires it. Finance postings own ledger truth; balances are projections recomputed by `finance_system::validate_record_transaction` and checked by invariants.
- Typed IDs and `EntityRef` carry identity; display text does not. Handle closed enums exhaustively and use typed errors. Keep external effects behind adapters and remove superseded internal paths unless an active external contract requires them.

## Tick, determinism, and persistence

- `core::simulation::run_tick` in `src/core/simulation.rs` is the sole authoritative minute. Speed changes invocation frequency, not tick semantics. Insert autonomous work into its explicit phase order with an ordering rationale.
- Preserve stable ordering and tie-breakers. Use owned `operation_rng_mut`, `investigation_rng_mut`, `business_rng_mut`, and `enterprise_rng_mut` streams through `core::simulation::draw_index`. Do not introduce ambient entropy or randomness solely to break ties. Preserve existing unconditional draws where matched-cycle determinism depends on them.
- Every future-affecting value survives `build_save → restore_save` in `src/core/persistence.rs`. State changes require serialization, invariant/load checks, and round-trip/continuation evidence. Derived indexes remain reconstructible from records.
- Incompatible schema/content fails closed under the current-version policy in [STATUS.md](STATUS.md). Preserve ID high-water marks, typed references, chronology, lifecycle relationships, and registry-derived invariants.

## Harness and extensions

`examples/gameplay_harness/` uses canonical production operations and its documented player-visible information. `[DEV AUDIT]` diagnostics never inform decisions. Acting policy uses actionable observations, not inferred safe windows from vague patrol text. [TESTING.md](TESTING.md) owns modes, matched-seed comparison, reports, and evidence limits.

For new domains, extend the architecture ownership map and existing private-state, canonical-operation, index, and invariant patterns. For authored kinds, update the closed vocabulary, registry, exhaustive consumers, and registry-dependent validation together. Do not add inert fields or variants. New tests exercise production operations with typed rejection, unchanged state, and explicit deterministic inputs.

## Completion

Use [TESTING.md](TESTING.md) and repository aliases/scripts; they own lockfile, profile, and incremental-build assumptions. Focused edit-loop checks are not mandatory predecessors to a completion lane that covers them.

- Documentation/routes: `.\scripts\check-docs.cmd` (compile-free).
- One-owner behavior: `.\scripts\verify.cmd -Fast -Filter <filter>`.
- Broader library behavior: `.\scripts\verify.cmd -Fast`.
- Harness implementation: `.\scripts\verify.cmd -Harness`.
- Persistence, invariants, cross-domain work, verification infrastructure, or an explicit broad checkpoint: `.\scripts\verify.cmd`.

Review the task diff for the affected ownership, determinism, persistence, invariant, adapter, and documentation contracts.
