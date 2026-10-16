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

The user is a solo developer. Precedence: current request → this contract → project card → standards. **Hard** rules survive lower-level workflow advice.

### AG-1 Finish the work

Finish the authorized task; plans, progress, or deferred notes are not delivery. On open-ended or hill-climbing goals, keep iterating—observe, diagnose, fix, verify—until the goal is met or in-scope improvements are exhausted; if one track is blocked, move to the next unblocked work in scope. Delegate only bounded read-only work when authorized.

### AG-2 Make the calls yourself

Make in-scope decisions yourself; state consequential choices briefly and continue. Do not ask permission for routine verification, bounded evaluations, or the next in-scope step; ask only when a choice changes the deliverable and cannot be inferred. Read the project card once, then resolve authority, owner, and proof. Expand only for unresolved scope or boundaries. Links and profiles are lookups; research and artifact work starts at the product workflow.

### AG-3 Stay in your project (hard)

Work only in the requested project. Do not create top-level directories or write to the workspace root or project parent. Put scratch work in the project's ignored output (normally `target/agent-output/`) or OS temp.

### AG-4 Done means the user's copy works (hard)

Verify every requirement against the user's actual copy. Refresh and check the affected release, deployment, or running app; source edits alone do not update it. Docs-only work verifies docs and routes. Use the smallest complete project lane and [`REVIEW.md`](../REVIEW.md) for consequential code. Claim only checked results; report what remains unverified.

### AG-5 Look at what you made

Run or render changed visual, audio, interactive, or published output; inspect it against [`STANDARDS_QUALITY.md`](../STANDARDS_QUALITY.md) and project references. Use isolated workbench or multi-angle views where detail matters, compare directly against references at matching angles and delivery scale, fix defects before reporting, and report rendered evidence. Internal prose needs only readability and route review. Tests do not prove appearance.

### AG-6 Real behavior over proxies

Observe user-experienced behavior. Tests, harnesses, validators, and metrics are evidence for hill climbing, not the goal. Use their findings to diagnose and fix systemic root causes in production owners; repair proxy/product divergence, and never weaken gates, hardcode outcomes, suppress warnings, or retry until green. Simulations stay causal and parameterized (STANDARDS_QUALITY.md QUAL-3).

### AG-7 Answer first, then stop talking

Answer questions directly; they are not permission to act. Reports state changes, verification limits, and required user action. Put long audits or research in a project file. No lectures or revisiting dismissed topics.

### AG-8 Own mistakes; trust the user's evidence

On a user-reported failure, re-check your work before blaming setup; own and fix mistakes. Try a tool, file, or capability before claiming it is unavailable.

### AG-9 The outcome outranks the process

Skip ceremony, optional diagnostics, or intermediate process when they cost more than they protect, and state what you skipped. Do not use this rule to skip the smallest complete verification lane (AG-4), applicable inspection of user-visible output (AG-5), or real-behavior evidence (AG-6). Hard rules still apply.

### AG-10 Leave nothing running

Run automation headlessly or in the background without popping up extra terminal or GUI windows that disrupt the desktop. Stop task-owned processes, servers, watchers, and terminals; release locks and remove scratch. Keep deliverables and the app the user should run. Never kill or replace the user's program silently. Built software cleans up child processes on exit.

### AG-11 Git: `main`, commit, push (hard)

Work on `main`; branches or worktrees only when asked. Commit and push verified work unless told not to. In a dirty shared tree, stage task-owned paths explicitly; broad add/commit-all modes MUST NOT sweep in unrelated changes. Never revert, stash, overwrite, or discard others' changes. Shared progress MAY be included only when deliberately selected, reviewed, and verified as complete; otherwise leave it untouched and report it.

### AG-12 Current docs describe the present

Update the single authority for changed behavior. Keep history, session notes, and changelog prose out of current docs/comments. Plans and design intent do not prove capability.

### AG-13 CI is local; GitHub Actions are banned (hard)

Use repository-owned local verification. Never create, enable, invoke, or push `.github/workflows/`; remove existing workflows while keeping their local equivalent.

### Shared workspace tools

Shared portfolio tools are available from this project root through `../tools/`; using them does not authorize edits outside this project. Start with:

```text
python ../tools/agent_context.py brief crimocracy <task ...>
python ../tools/agent_context.py --help
python ../tools/agent_context.py diff-impact crimocracy
```

`agent_context.py` provides `brief`, `context`, `impact`, `diff-impact`, `outline`, `structure`, `architecture`, `backends`, `audit`, `rule`, and `exemplar`. `brief` returns copyable next commands for the current working directory.
For Cargo projects, `python ../tools/rust_agent_tools.py check` verifies the shared Rust diagnostics; use them only as routed by this card's Rust diagnostics marker and `TESTING.md`.
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
