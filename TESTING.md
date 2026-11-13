# Testing

Owns test selection, text play evidence, and local verification. Ownership is in
[`ARCHITECTURE.md`](ARCHITECTURE.md); scope is in [`STATUS.md`](STATUS.md);
cockpit routing is in [`AGENTS.md`](AGENTS.md).

## Test selection — narrowest proof first

| Change | Focused feedback | Completion lane |
|---|---|---|
| Documentation/routes only | `.\scripts\check-docs.cmd` (compile-free) | same command |
| Syntax / types | `cargo check-fast` (no linking) | `.\scripts\verify.cmd -Check` (adds fmt) |
| One library behavior | `cargo test-focused <filter>` | `.\scripts\verify.cmd -Fast -Filter <filter>` |
| Library implementation | `cargo check-fast` or focused test | `.\scripts\verify.cmd -Fast` |
| Text play client | `cargo test-harness text_play` and `cargo text-play --seed 12` | `.\scripts\verify.cmd -Harness` plus a real text-play session |
| Persistence, invariants, or cross-domain | Owning module's focused test plus a save/restore round-trip | `.\scripts\verify.cmd` (broad gate) |
| Mixed-state stress | `cargo soak` (always explicit, never part of a gate) | same command |

The columns are not a required sequence. Use focused feedback while iterating or
isolating a failure; go directly to the completion lane once it compiles and
exercises the same owner coverage. Never rerun the broad gate after a passing
fast lane "for reassurance".

## Rust agent diagnostics

Optional Rust diagnostics may be used before selecting or strengthening the normal proof:

[`scripts/rust-diagnostics.ps1`](scripts/rust-diagnostics.ps1) (or its `.cmd` wrapper) is the
preferred entry point for repeated agent use because it pins the bounded project defaults below
and gives each mutation execution a unique ignored output directory. The underlying Cargo
commands remain useful when a task needs a flag the wrapper intentionally does not generalize.

- `cargo modules structure --lib --no-fns --no-traits --no-types --max-depth 4` is a bounded ownership map when the cockpit/architecture route still leaves the responsible module unclear. Prefer a crate-qualified focus such as `--focus-on crimocracy::<module>`. For unlinked-file checks use `cargo modules orphans --lib --cfg-test`; this repository keeps many tests in split `tests.rs` modules, so omitting `--cfg-test` produces false orphan reports. Do not gate on global cycle or orphan counts.
- `cargo mutants --list --file <owner.rs>` is the cheap first step when a consequential state transition, invariant, or persistence test may not actually distinguish wrong behavior. Execute only a narrow file/function/line selection when the result can change the test design. Prefer passing the owning test substring after `--` when one exists; that makes hangs and misses attributable to the contract under review instead of unrelated suites. The wrapper enforces a narrow regex, defaults to two jobs, and creates a unique `target/agent-output/mutants/<task>/mutants.out`; checked-in [`.cargo/mutants.toml`](.cargo/mutants.toml) keeps Cargo locked. Investigate missed or timed-out mutants individually. If a missed mutation is unreachable for every valid `AppState` because canonical construction and release-safe invariants already make the altered branch equivalent, record that reasoning instead of manufacturing invalid private state solely to kill it. Mutation score is not a completion target.
- `cargo expand --lib <module::item>` is an inspection aid for a derive/proc macro whose generated code is material to the task. The wrapper bounds expansion output to 200 lines by default and accepts `-Filter <regex> -Context N -MaxLines N` for a relevant generated region. If selecting a type shows only its declaration, expand the containing module and filter for the type or generated impl there. Expanded text is debugging evidence, not source or a compilation contract; call raw `cargo expand` only when an intentional full dump is actually needed.

Use these at the point of uncertainty, not as a ritual preflight: `Modules` when ownership remains
ambiguous after the architecture route, `MutantsList`/`Mutants` when a consequential focused test
may not distinguish nearby wrong behavior, and `Expand` when generated code is itself material to
the edit. A useful diagnostic changes where you read, what you test, or what you implement.

None of these replaces `.\scripts\verify.cmd` or a focused behavioral proof. Mutation execution stays in its isolated copy; do not use `--in-place` on a dirty/concurrent repository to bypass scratch-copy problems, and do not reuse another run's output directory.

## Test rules

Tests prove observable production behavior: calculations, transitions, transactions,
invariants, serialization, deterministic continuation, and failure paths.

- Exercise canonical system operations, not private helpers or test-only mutation shortcuts.
- Assert typed error variants and relevant fields, not rendered text.
- For atomic rejection, assert authoritative state is unchanged.
- Use explicit seeds and stable ordering. Do not hunt for a passing seed.
- Keep content-count, CRUD, or smokes only when they protect a real contract.
- Keep assertions non-brittle: pin counts only for authored bounds (brief entry caps,
  overflow arithmetic), derive expected display prose from the test's own production
  values (e.g. format the asserted cents through the production money formatter)
  instead of duplicating literals, and assert a coarse marker plus the typed record
  rather than a full sentence when the sentence is not the contract. Formatter
  output itself is pinned only in the formatter's own tests; consumers assert the
  typed signal and at most one coarse marker. Content values (prices, margins)
  are pinned only where the registry test is the deliberate content contract;
  behavior tests derive from the registry instead of duplicating its numbers.
- Persistence tests distinguish authoritative bytes from derived runtime indexes: save bytes omit
  owner-maintained lookup/scheduling projections, and restore must rebuild them before indexed reads.

Ordinary tests live with their owning module under `#[cfg(test)]`, named after
behavior. Use `make_test_*` for local fixtures and the idempotent `*_for_test`
pattern when extending the shared production registry. Soak-class tests carry the
substring `soak`.
cannot silently drop deep coverage. These deeper tiers are evidence, not prerequisites for unrelated edits.

### Accretion checklist for a new test

- [ ] Calls the owner's `validate_* → commit` or `decide_* → apply_*`, not a private helper.
- [ ] Asserts the typed `Error` variant (e.g. `FinanceError::InsufficientFunds`) + fields on failure, not a string.
- [ ] On rejection, clones `state` before and `assert_eq!(state, before)` after.
- [ ] Uses `AppState::new(explicit_seed)` and `BTreeMap`/`BTreeSet` ordering — no `HashMap` iteration.
- [ ] Named `fn <behavior>_when_<condition>()`, not `fn test_crud()`.

## Local verification — solo, in this repo, no hosted CI

### Fast lanes — inner loop

| Need | Command | What it proves |
|---|---|---|
| Docs/routes only | `.\scripts\check-docs.cmd` | authorities, links, routes, aliases, published versions; no Rust build |
| Type-check lib | `cargo check-fast` | `src/` compiles |
| Type-check all | `cargo check-all` | lib + text play client compile |
| Type-check client | `cargo check-harness` | text play example compiles |
| Lib tests (no soak) | `cargo test-fast` | all library tests except soak-class tests |
| One test / module | `cargo test-focused <filter>` | owning module's `#[cfg(test)]` |
| Client contracts | `cargo test-harness` | text play command and session tests |
| One client behavior | `cargo test-harness <filter>` | matching client tests only (check the pass count; a typo matches zero) |
| Auto-rerun on save | `.\scripts\watch.cmd` (`-Filter`, `-Harness`, or both) | lib check by default; lib focused or client focused on demand |
| Check lane | `.\scripts\verify.cmd -Check` | fmt (changed files) + lib type-check |
| Fast lane (fmt + lib) | `.\scripts\verify.cmd -Fast` | library completion gate; fmt checks changed files only |
| Text play lane | `.\scripts\verify.cmd -Harness` | fmt + text-play client command contracts |
| Filtered fast lane | `.\scripts\verify.cmd -Fast -Filter <pat>` | fail-closed focused lib tests (soak excluded unless the filter contains `soak`) + fmt (changed) |
| Filtered text play lane | `.\scripts\verify.cmd -Harness -Filter <pat>` | fail-closed focused client tests + fmt (changed) |
| Soak only | `cargo soak` | mixed-state invariant stress |

`cargo check-fast` is the absolute fastest; `cargo test-focused` is the inner loop
for library behavior and `cargo test-harness <filter>` is the inner loop for the
text play client. A single-owner change can complete through the filtered fast
lane without rerunning every library test. A client change can complete through
the filtered text play lane without rerunning the whole client suite. The broad
gate is reserved for contracts that actually cross persistence, invariant,
domain, or verification boundaries. Real player-experience judgement comes from
playing a session, not from a gate.

Build-profile tuning and measured compile-cost observations live with Cargo configuration.
This document owns behavioral proof selection, not machine-specific timing claims.

### Broad completion gate — when cheap lanes are not enough

```text
.\scripts\verify.cmd
.\scripts\verify.cmd -Jobs 2
.\scripts\verify.cmd -Check          # type-check only, fastest gate
.\scripts\verify.cmd -Fast -Filter payroll   # one module
```

Fail-fast stages, in order (see [`scripts/verify.ps1`](scripts/verify.ps1)):

1. Compile-free documentation/route/alias/version contracts (`scripts/check-docs.ps1`; skipped when no doc/version files changed)
2. fmt on changed `.rs` files (full scan only for a clean checkpoint tree or unreadable git state)
3. `cargo test --locked --lib --quiet -- --skip soak` (soak stays explicit via `cargo soak`; filtered lanes also exclude soak unless the filter contains `soak`)
4. Text play client command contracts (`cargo test --example gameplay_harness`; skipped when no `src/`/client files changed)
5. `cargo clippy --locked --lib --example gameplay_harness -- -D warnings`

Skips are fail-closed and printed as `SKIP` lines with reasons: unreadable git
state or a clean checkpoint tree runs every stage.

[`scripts/verify.ps1`](scripts/verify.ps1) owns the broad gate; [`scripts/verify.cmd`](scripts/verify.cmd) wraps it. The broad gate proves the client still drives canonical operations instead of rerunning its whole command suite: client contracts stay in the text play lane so persistence or cross-domain checkpoints do not rebuild and rerun the example test binary. `scripts/check-docs.ps1` replaces the former Rust integration test for authority/link/route/alias/version checks, so documentation-only completion does not compile the crate.

**When to run what:**

- One-owner behavior work may complete with `.\scripts\verify.cmd -Fast -Filter <filter>`; broader library work with `.\scripts\verify.cmd -Fast`; text play client work with `.\scripts\verify.cmd -Harness` (or `-Harness -Filter <pat>` for one behavior).
- Run the broad gate only when persistence, invariants, cross-domain behavior, verification infrastructure, or another changed contract requires its wider client/Clippy coverage, or for an explicit broad checkpoint. It intentionally stops at the client's command contracts; real play observation is specialized evidence, not a tax on unrelated changes. Never rerun it after a passing narrower lane merely for reassurance.
- Run `cargo soak` only when the changed contract requires that evidence.
- When optimized compilation could change behavior, also run `cargo test-release`.

Gate flags: `-Check` (fmt on changed files + lib type-check) | `-Fast` (library tests, soak excluded; fmt on changed files) | `-Harness` (client command contracts; with `-Filter <pat>` runs fail-closed focused client tests) | `-Filter <pat>` (fail-closed focused tests; implies `-Fast` unless `-Harness` is given) | `-Jobs N` (cap parallelism) | `-NoClippy` / `-NoFmt` (skip known-passing) | `-Verbose` / `-Detail` (show captured output on success).

Documentation contracts are intentionally outside Rust's integration-test graph. The checker reads source declarations directly, so Markdown and command-surface changes have a complete compile-free path and the broad gate no longer links a second test binary merely to validate documentation.

## Text play — primary player-experience surface

Use `cargo text-play` to play through decisions and consequences without an
automated evaluation policy selecting the player's moves. This is a thin
terminal adapter under `examples/gameplay_harness/text_play/`; it shares the
canonical authored starting scenario in `scenario.rs` but does not run any
automated strategy or rebuild that fixture during save restoration. Its
`mod.rs` owns session creation, current-version saves, options, and the
player-scoped read-only selection rules; `commands.rs` dispatches interactive
player choices, `governance.rs` delegates policy/mandate mutation to the
production delegation owner, and `planning.rs` composes registry-directed
operation drafts. `tests.rs`, `governance_tests.rs`, `openings_tests.rs`, and
`legal_planning_tests.rs` prove actual tick continuation, player information
boundaries, saving, authorization, and rejection integrity.
`run_tick` remains the only way to advance simulation time.

Views show public business-directory identities (not ownership), organization-held
intelligence, its own finances, people, businesses and enterprises, delivered
reports, and pending decisions. Public business names do not themselves become
intelligence. The CLI must not read hidden legal cases or rival records to
recommend actions. Role assignments come from current organization members;
no participant/authority IDs from the starting fixture are retained. Entity
selection checks organizational ownership before producing actionable plans.
Opening accounts, same-kind treasury allocation, laundering, recruitment,
business acquisition, enterprise establishment and lifecycle, burglary and
reconnaissance, discovery, disposition, and decision resolution all use canonical
owner paths. Generic money movement is not a laundering or minting shortcut.
The `--opening` option selects one of six deterministic authored alternatives;
`--load` excludes all seed/opening overrides and restores its saved campaign.
`operation-kinds` derives actual role/approach/duration requirements from the
registry. `plan` uses the current player roster, rejects automatic assignment
of untrained specialists, and accepts explicit role/leader choices. Foreign
businesses require organization-held target information; person-targeted
extraction and witness pressure require identified characters, while canonical
authorization enforces typed, episode-bound learned legal-status facts. A
general observation about a person is not evidence of witness status or arrest.

Leadership pauses for pending decisions, enabled urgent-report attention, and
terminal outcomes of its own operations. The production save envelope preserves
campaign state and reconstructs derived indexes on restoration.

Start with `cargo text-play --seed 12` for reproducibility and use `help` for
the available actions. Script input is one command per line, with blank lines
and lines beginning `#` ignored. Interactive command rejection prints a
reason without ending the session; a rejected scripted command fails with its
line number. Excess or malformed command arguments are rejected before any
mutation. The currently playable opening is an authored scenario, not general
campaign generation. An initial crew is authored, but current crew membership
and skills determine available orders and recruitment changes that roster.
The CLI still covers only business and legally identified person targets; other
valid operation target forms, organic campaign generation, and proactive legal
intelligence acquisition remain outside this adapter's current command surface.

Text play is the primary judge of comprehension, usability, and campaign
pacing. Do not reintroduce a scripted policy that selects the player's moves:
that duplicates the game's rules in the harness and drifts from the shipped
game. Player-behavior judgement comes from reading a real session and its
reports. Text-play command tests are included in the `cargo test-harness`
target.

## Completion checklist

Before handoff:

- [ ] Ran exactly the smallest scripted completion lane that covers the changed surface (see decision tree above); did not rerun an overlapping focused proof immediately before that lane.
- [ ] `git diff --check` is clean; generated output and final worktree reviewed.
- [ ] If a test, client command, alias, or verification rule changed, this document and the owning script/command definition were updated in the same change.

When a test, client command, alias, or verification rule changes, update this document and the owning script or command definition in the same change.
