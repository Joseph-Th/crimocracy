# Agent Guide — Crimocracy Cockpit

**BCA policy:** advisory

**Rust agent diagnostics:** advisory. Use [`scripts/rust-diagnostics.ps1`](scripts/rust-diagnostics.ps1) for bounded cargo-modules structure views when the architecture map still leaves an owner ambiguous, targeted cargo-mutants selection/execution when focused tests may not constrain a consequential state/tick policy, and cargo-expand only when macro/derive output is material. The wrapper gives mutation runs task-unique ignored output by construction. These diagnostics are review evidence, not completion gates or quality scores. [TESTING.md](TESTING.md) remains the verification authority.

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

The user is a solo developer. Precedence: the user's current request → this contract → the project's `AGENTS.md` → the standards reference. Rules marked **hard** hold even when the user says to ignore the workflow.

### AG-1 Finish the work

Finish the task before reporting. Don't stop at a plan or a progress update, and don't defer requested work into notes. Delegate only bounded, read-only work to subagents.

### AG-2 Make the calls yourself

Design and implementation decisions inside the task are yours. Choose the approach a strong senior engineer would choose, state the choice in one line, and keep going. Ask only when the answer would change what the user gets and you cannot infer it from the request, the code, or common sense.

### AG-3 Stay in your project (hard)

Work only inside the project the task is about. If your session starts at the workspace root, identify that project from the request and work there. Never create top-level directories, and never write output to the workspace root or a project's parent. Temporary output goes in the project's ignored output location (normally `target/agent-output/`) or the OS temp directory.

### AG-4 Done means the user's copy works (hard)

Before you say done:

- Re-read the original request and check every requirement against the actual result by running it, opening it, or looking at it.
- Rebuild the release binary, redeploy the site, or restart whatever the user will actually run, and say that it's current.
- For consequential code changes, go through [`REVIEW.md`](../REVIEW.md) against your diff.

Never claim something is fixed, verified, deployed, or certain unless you checked it. Report what you did not verify.

### AG-5 Look at what you made

For anything seen or heard (graphics, UI, charts, animation, audio, documents), render it and inspect it yourself against the matching rubric in [`STANDARDS_QUALITY.md`](../STANDARDS_QUALITY.md) before calling it done.

- Look at individual assets up close and from several angles, not a crowded overview.
- Compare against the references.
- Check that every output file was actually produced.
- Keep iterating until you would be proud to show it.

Include the screenshots or rendered files in your report. Tests prove behavior, not appearance.

### AG-6 Real behavior over proxies

A passing harness, test, validator, or metric is evidence, not the goal. The goal is the behavior the user experiences. Observe it directly and judge it with common sense against how the real world works. Fix a harness that diverges from the product rather than tuning the product to the harness. Never make a gate pass by weakening it, hardcoding the expected outcome, suppressing warnings, or retrying until green. Simulations get emergent, parameterized systems, not scripted outcomes (STANDARDS_QUALITY.md QUAL-3).

### AG-7 Answer first, then stop talking

When the user asks a question, the first sentence answers it directly. Don't act on a question as though it were a request. Reports are short:

- what changed;
- what you verified and how;
- what you did not verify;
- anything the user must do.

No lectures, recaps, hedging paragraphs, or repeated caveats, and never keep raising a topic the user has dismissed. Put long material (audits, research, data) in a file in the project and link it.

### AG-8 Own mistakes; trust the user's evidence

When the user says something is broken, believe them and re-check your own work before suspecting their setup. Say plainly when you were wrong, then fix it. Never claim a tool, file, or capability is unavailable without actually trying it.

### AG-9 The outcome outranks the process

Standards and workflows exist to make results better. When a documented procedure would make the requested result worse, or cost far more than it protects, favor the result and note in one line what you skipped. This never overrides a hard rule.

### AG-10 Leave nothing running

Before you finish, stop every process, server, watcher, and terminal you started. Release file locks, and delete temporary builds and scratch files you created. Software you build must not leave orphaned child processes when it closes. Never kill or replace a program the user is running without saying so.

### AG-11 Git: `main`, commit, push (hard)

Work on `main`, and don't create branches or worktrees unless asked. When the work is complete and verified, commit and push `main` unless the user said not to. Other agents' changes in the tree may go in with yours when they are sound progress. Never revert, stash, or discard work you did not make; leave out anything clearly broken that isn't yours, and mention it.

### AG-12 Current docs describe the present

When behavior changes, update the one document that owns that fact. No history, war stories, changelogs, or session notes in current docs or comments. Design and roadmap documents are not proof that something is implemented.

### AG-13 CI is local; GitHub Actions are banned (hard)

All builds, tests, checks, and audits run through repository-owned local commands. Never create, enable, invoke, or push `.github/workflows/`; existing workflow files are defects to remove.
<!-- workspace-contract:end -->

## 1. At a glance

```text
Registry (immutable, build_registry)  ─┐
AppState  (mutable domain state + 4 RNG streams + clocks) ─┤─► run_tick (1 min, deterministic ordered phases)
Harness   (evaluation surface, smoke/full, player-visible only) ─┘
```

- **You mutate through one owner per field** via `validate_* → Validated*::commit` or
  `decide_* → apply_*`. Never construct a `*Record{}` directly.
- **One tick = one minute** (`core::simulation::run_tick` at `src/core/simulation.rs`).
  Speed is an adapter concern — call it more often, don't change its semantics.
- **Determinism** = `Registry + AppState + ordered inputs + state-owned RNG`. No wall
  clock, no hash iteration, no ambient entropy.
- **Verification:** use the narrowest focused check while editing and the completion lane
  selected by [`TESTING.md`](TESTING.md). Cross-domain, persistence, invariant, or
  verification-infrastructure work requires the broad local gate.

---

## 2. System tower — the single mental model

The codebase is a tower of linked abstractions, not a bag of modules. The tower describes
ownership and orchestration responsibility rather than a literal Rust import DAG: lower-level
owners do not patch higher-domain state, while orchestrating domains may use peer read APIs and
validated integration paths where the model requires feedback.

```text
Layer 4 — Orchestrating domains (transact across many owners)
  operations  legal  enterprises  economy
       \        |        |         /
        \       |        |        /
Layer 3 — Mid domains (cross-reference, own mutations)
  finance  delegation  reputation  decisions  contacts  opportunities  recruitment
              \         |            |           |           |            |
Layer 2 — Foundational domain owners (low-dependency state)
  world  social  intelligence  history  reports
              \    |        |         |       /
Layer 1 — Immutable authoring
  registry  ◄──  content::build_registry  (content::CURRENT_CONTENT_REVISION)
               \
Layer 0 — Foundations (everyone depends on these)
  core::{id, time, entity, attention, state, simulation, persistence, invariants}
```

The authoritative dependency policy, `AppState` ownership map, persistence model, and
source map live in [`ARCHITECTURE.md`](ARCHITECTURE.md). `src/lib.rs` is the
authoritative top-level module inventory.

---

## 3. Canonical operations — quick reference

Every consequential mutation follows one of two shapes. Treat the `//!` header of
each `src/*/mod.rs` as contractual — its second line names the owning system.

### Pattern A — Validate then commit (fallible multi-record)

```text
validate_*(&state, …) -> Result<Validated*, TypedError>
Validated*::commit(self, &mut state) -> Result<Outcome, TypedError>
```

Validation resolves references, permissions, lifecycle, ownership, capacity, ranges,
and arithmetic **before** any mutation. Commit rechecks freshness (`SimTime` staleness
via `ensure_time_current`) and preserves indexes atomically. Rejected operations leave
authoritative state unchanged.

Find the owner in the [`ARCHITECTURE.md`](ARCHITECTURE.md) source map, then read that
module's `//!` header and focused tests. Do not maintain a second entry-point inventory here.

### Pattern B — Decide then apply (read-only derivation)

```text
decide_*(&state, …) -> Plan / Outcome / Delta    // read-only, may take &mut RNG
apply_*(&mut state, plan)                        // single-owner, preserves invariants
```

Decision reads broader state than it mutates. Randomness is explicitly supplied
(`&mut ChaCha8Rng`) and drawn via `core::simulation::draw_index`.

### Single-owner direct mutation

Allowed only when every return path preserves that owner's invariants and indexes
(`social::relationship_system::set_relationship`, `reputation_system::apply_reputation_delta`).

**Rule:** adapters, tests, examples, importers, and tools use the same owner methods.
No bypasses. If you are constructing a `*Record` literal, stop — use the owner's
`Draft → validate → commit` path.

---

## 4. Change recipe — 7 concrete steps

1. **Locate the owner.** Find the `AppState` field (`src/core/state.rs`) and the
   `ARCHITECTURE.md` source map. Read the `//!` header of that `src/*/mod.rs`. If the
   route is still ambiguous inside a large domain, use bounded `rust-diagnostics Modules`
   with a crate-qualified `-Focus` before widening source search.
2. **Read the focused tests.** Open `src/<owner>/tests.rs` first, falling back to `#[cfg(test)]`
   in the owning `*_system.rs`. They are the executable contract. For a consequential branch
   whose assertions may not distinguish nearby wrong behavior, use `MutantsList` and then a
   narrow `Mutants` selection to decide whether a regression test is actually missing; do not
   sweep unrelated code or optimize a mutation score.
3. **Name the canonical operation.** Search for `validate_*` or `decide_*` in the
   owner's `*_system.rs`. Do not invent `create_*`/`make_*`/`execute_*`.
4. **Implement through the owner.** Validate before mutation; commit atomically;
   maintain every derived index (`BTreeMap` + `BTreeSet`) and bump `version` where
   present. Handle the project's enums exhaustively.
5. **Preserve determinism.** Use `BTreeMap`/`BTreeSet` or explicit stable sorting
   with tie-breakers. When authored behavior is stochastic, draw only from
   `state.operation_rng_mut()` / `investigation_rng_mut()` / `business_rng_mut()` /
   `enterprise_rng_mut()` via `draw_index`. Do not add randomness merely to break ties.
6. **Preserve persistence.** Every future-affecting value must survive `build_save` →
   `restore_save` (`src/core/persistence.rs`). Add `#[derive(Serialize,Deserialize)]`
   and a round-trip test if you add state. When correctness depends on a macro or derive you
   cannot verify from source syntax alone, use bounded `rust-diagnostics Expand` on the
   containing module with `-Filter` rather than guessing what was generated.
7. **Prove it with the narrowest lane** in [`TESTING.md`](TESTING.md), then run the
   completion lane required for the changed surface.
   Update the single authority document whose contract you changed.

If any step is not discoverable, repair the owning documentation as part of the change.

---

## 5. Verification lanes — which command when

[`TESTING.md`](TESTING.md) owns the decision tree, exact gate stages, harness modes, and
which lane completes each change class. Use focused checks while editing when they
shorten feedback. The broad local gate is required for persistence, invariant,
cross-domain, or verification-infrastructure changes.

`scripts/check-docs.ps1` mechanically checks the live documentation routes, Cargo aliases,
and persistence-version publication rules without compiling the crate.

---

## 6. Determinism · persistence · invariants — checklist

Before handoff, every state-affecting change must preserve all three:

- **Determinism:** ordered collections or stable sorting with tie-breakers; no ambient wall
  clock, filesystem order, thread scheduling, hash iteration, or external entropy as input;
  result-affecting randomness comes only from serialized `AppState` RNG streams.
- **Persistence:** every future-affecting authoritative value survives save/restore; derived
  indexes remain projections rebuilt from authoritative records; current-version-only loading
  rejects incompatible state rather than defaulting or migrating it silently.
- **Invariants:** owner indexes, typed references, lifecycle relationships, ID high-water marks,
  timestamps, and registry-derived values remain re-derivable and valid.

[`ARCHITECTURE.md`](ARCHITECTURE.md) owns the detailed RNG, persistence, and invariant
contracts. [`TESTING.md`](TESTING.md) owns the proof lanes.

---

## 7. Runtime flow — the authoritative tick

`core::simulation::run_tick` (`src/core/simulation.rs`) is the **only**
authoritative minute. Phase order is contractual. [`ARCHITECTURE.md`](ARCHITECTURE.md)
owns the phase diagram; `src/core/simulation.rs` owns executable order and rationale
comments. New autonomous work must slot explicitly there with an ordering rationale.

---

## 8. Harness — bounded evaluation surface (not a playtest)

`examples/gameplay_harness/` exercises **production paths** through player-visible
information only. `[DEV AUDIT]` is diagnostic, never fed to decisions.
[`TESTING.md`](TESTING.md) owns harness modes, evidence contracts, artifacts, and
matched-seed comparison rules. Do not infer safe operation windows from vague patrol
text; acting policy must use only actionable player-visible information.

---

## 9. Common pitfalls — fail fast, not silently

| # | Anti-pattern | Why it breaks | Correct path |
|---|---|---|---|
| 1 | Constructing `*Record{}` or patching a private field | Skips validation, indexes, version bumps; `validate_state` fails | Use `*Draft → validate_* → commit` via the owning `*_system` |
| 2 | `HashMap`/`HashSet` or unsorted `Vec` for order-sensitive work | Nondeterministic iteration → flaky `soak` and harness divergence | `BTreeMap`/`BTreeSet` or explicit stable sort + tie-breaker |
| 3 | `rand::thread_rng()` or `SystemTime::now()` | Ambient entropy leaks into authoritative state | `state.<domain>_rng_mut()` + `draw_index` (rejection sampling) |
| 4 | Adding a record but forgetting its schedule/index | Invisible to `run_tick`; leaks onto revoked/suspended work | Follow the 3-part pattern: `BTreeMap::insert` + derived `BTreeSet` + `has_consistent_indexes` (see `src/finance/mod.rs`) |
| 5 | Forgetting `IdCounters::reserve` before multi-record commits | Allocator high-water mark drifts, `validate_id_allocators` fails on next save | Call `state.ids.reserve(…)` before any `validate_*` that may allocate |
| 6 | Consuming RNG conditionally (e.g. enforcement roll only when racket inquiry) | Branches needing matched determinism diverge | Draw unconditionally per cycle (see `src/core/simulation.rs`) |
| 7 | Writing ledger `balance` directly | Balance is a materialized view; ledger `postings` are the truth | `finance_system::validate_record_transaction` derives balances; audit re-derives via dense `Vec<i64>` at `src/core/invariants/finance.rs` |
| 8 | Using display text as identity | Fragile foreign keys, collisions | Typed IDs (`CharacterId`, `BusinessId`) + `EntityRef` where project controls vocabulary |
| 9 | Silently defaulting a missing future-affecting value on load | Old save loads but loses continuation fidelity | Incompatible schema/content is rejected; [`STATUS.md`](STATUS.md) owns the current compatibility policy |
| 10 | Bypassing repository Cargo aliases or verification scripts | Can silently change lockfile, profile, or incremental-build assumptions | Use the aliases in `.cargo/config.toml` or the scripted gate in [`TESTING.md`](TESTING.md) |

---

## 10. Accretion guide — adding without fragmenting

For a new domain, first place it in the dependency tower and ownership map in
[`ARCHITECTURE.md`](ARCHITECTURE.md), then follow the nearest existing owner pattern for
private state, canonical mutation, indexes, invariants, persistence, and any required
state-owned randomness. Autonomous behavior must be inserted explicitly into `run_tick`.

For a new authored kind, update the closed vocabulary, authored registry definition,
all exhaustive matches, and registry-dependent invariant re-derivation. Do not add inert
fields or variants without a consuming system.

For a new test, exercise the canonical operation, assert typed failures and unchanged
state on rejection, use explicit seeds/stable ordering, and follow [`TESTING.md`](TESTING.md).

---

## 11. Authority map & reading order

| Question | Authority |
|---|---|
| Repository and collaboration rules | Workspace `AGENTS.md` (if present) |
| Project execution rules | **This file** (cockpit) |
| State ownership and mutation | [`ARCHITECTURE.md`](ARCHITECTURE.md) |
| Implemented scope and exclusions | [`STATUS.md`](STATUS.md) |
| Tests and harness evidence | [`TESTING.md`](TESTING.md) |
| Product intent | [`GAME_DESIGN.md`](GAME_DESIGN.md) |
| Commands and local gate | [`README.md`](README.md) and [`TESTING.md`](TESTING.md) |
| Executable behavior | Owning `src/` module and its focused tests |

**Cold start — read in order:**

1. If the repo lives in a portfolio workspace, read the workspace `../AGENTS.md` first.
2. This file (§1-§5 for the working loop; §6-§10 as needed).
3. [`STATUS.md`](STATUS.md) — what exists and what is explicitly excluded.
4. [`ARCHITECTURE.md`](ARCHITECTURE.md) — contracts, then the `//!` header of the
   owning `src/` module and its focused tests before editing.
5. [`TESTING.md`](TESTING.md) before changing tests, persistence, or harness behavior.
6. [`GAME_DESIGN.md`](GAME_DESIGN.md) only for product-intent questions.

If authorities conflict, the owning contract and implementation win. Repair stale
wording as part of the change.

## Non-negotiable rules

- One owner per consequential state field; mutate only through the canonical production path.
- Tests, examples, adapters, importers, and tools use owner methods. No bypasses or mutation shortcuts.
- Validate fallible multi-record operations before mutation. Rejected operations leave authoritative state unchanged unless the contract explicitly records a failure.
- Keep ordering and randomness deterministic: ordered collections or explicit stable sorting with tie-breakers, state-owned RNG only.
- Persist every future-affecting runtime value; cover with invariant, load, and continuation checks where applicable.
- Handle project-owned enums exhaustively; use typed error enums for new fallible operations.
- Keep external effects (filesystem, network, UI, process) behind explicit adapter boundaries.
- Delete superseded paths. Do not keep historical shims.
- Keep documentation concise, current, and forward-facing. No implementation diaries.
- Verification is local. Do not add or depend on GitHub Actions.

## Completion

Use focused checks while editing when they shorten feedback or isolate a failure. For completion, run the smallest lane that covers the changed surface; if the implementation is already ready for that lane, go directly to it instead of forcing a focused build first:

- `cargo check-fast` / `cargo test-focused <filter>` while editing
- `.\scripts\check-docs.cmd` for documentation/route-only changes
- `.\scripts\verify.cmd -Fast -Filter <filter>` for one-owner behavior work
- `.\scripts\verify.cmd -Fast` for broader library work
- `.\scripts\verify.cmd -Harness` when the harness implementation changes
- `.\scripts\verify.cmd` only for persistence/invariant/cross-domain work, verification infrastructure, or an explicit broad checkpoint

Before handoff, confirm ownership, determinism, persistence, invariants, adapters, tests, documentation, and worktree scope remain coherent.
