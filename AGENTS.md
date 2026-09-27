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

The user is a solo developer. Precedence: the user's current request → this contract → the project card → standards references. **Hard** rules cannot be waived by lower-level workflow advice.

### AG-1 Finish the work

Finish the authorized task; a plan, progress update, or deferred note is not delivery. Delegate only bounded, read-only work when delegation is authorized.

### AG-2 Make the calls yourself

Make design and implementation decisions within scope; state consequential choices briefly and continue. Ask only when the answer changes the result and cannot be inferred. Read the project card once, then the task's authority, owner, and proof route. Begin when those are clear; expand for unclear scope or crossed boundaries. Links and profiles are lookups, not a recursive reading list. Use product workflows before internals for research or artifact authoring.

### AG-3 Stay in your project (hard)

Identify the requested project before working. Do not create top-level directories or write output to the workspace root or a project's parent. Scratch work belongs in the project's ignored output location (normally `target/agent-output/`) or OS temp.

### AG-4 Done means the user's copy works (hard)

Check every requested requirement against the actual result. Refresh and verify the affected release binary, deployed site, or running application the user uses; source edits alone do not update it. Documentation-only work verifies the delivered documents and routes. Use the smallest complete project verification lane, and [`REVIEW.md`](../REVIEW.md) for consequential code changes. Claim only what you checked; report unverified requirements.

### AG-5 Look at what you made

Render or run changed visual, audio, interactive, or published output and inspect it against the applicable [`STANDARDS_QUALITY.md`](../STANDARDS_QUALITY.md) rubric and references. Inspect individual assets at useful scales and angles, verify every requested file, and fix defects. Include rendered evidence in the report. Internal prose needs readability and route review only, not a publication workflow. Tests do not prove appearance.

### AG-6 Real behavior over proxies

Observe the behavior the user experiences. A passing test, harness, validator, or metric is evidence, not the goal. Repair harness/product divergence; never pass a gate by weakening it, hardcoding outcomes, suppressing warnings, or retrying until green. Simulations use causal, parameterized systems (STANDARDS_QUALITY.md QUAL-3).

### AG-7 Answer first, then stop talking

Answer questions directly; do not treat them as permission to act. Reports state what changed, verification and its limits, and any required user action. Put long audits or research in a project file and link it. No lectures or revisiting dismissed topics.

### AG-8 Own mistakes; trust the user's evidence

When the user reports a failure, re-check your work before blaming their setup. Own mistakes plainly and fix them. Try a tool, file, or capability before claiming it is unavailable.

### AG-9 The outcome outranks the process

If a procedure harms the requested result or costs more than it protects, favor the result and briefly state what you skipped. This does not waive hard rules.

### AG-10 Leave nothing running

Stop task-owned processes, servers, watchers, and terminals; release locks and remove your scratch output. Keep requested deliverables and the updated application the user is meant to run. Never kill or replace the user's program without saying so. Built software must clean up its own child processes on exit.

### AG-11 Git: `main`, commit, push (hard)

Work on `main`; create branches or worktrees only when asked. Commit and push verified work unless the user says not to. Preserve others' changes: never revert, stash, or discard them. Include sound shared progress when appropriate; leave clearly broken unrelated work out and report it.

### AG-12 Current docs describe the present

Update the single authority for changed behavior. Keep history, session notes, and changelogs out of current docs and comments. Plans and design intent do not prove implemented capability.

### AG-13 CI is local; GitHub Actions are banned (hard)

Use repository-owned local build, test, check, and audit commands. Never create, enable, invoke, or push `.github/workflows/`; remove existing workflow files while retaining their local verification equivalent.
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
