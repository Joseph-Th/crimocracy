# Tasks

Current executable work that may be picked up asynchronously. Keep entries short; remove completed work. `STATUS.md`/`CAPABILITIES.md` owns implemented truth and `ROADMAP.md`/`DIRECTION.md` owns future direction.

## Completed

- **T-0001 — Fail closed when smoke gate selects zero tests** (scripts/verify.ps1): `verify.ps1` now lists the gameplay harness's ignored tests before running stage 4 and requires that exactly `tests::smoke_mode_covers_canonical_paths` is selectable, so a renamed/removed contract fails the gate instead of silently erasing the stage. A `-SelfTest` switch runs the present/missing/ambiguous/zero regression.
- **T-0002 — Make persistent ID exhaustion recoverable and atomic** (src/core/id.rs, all owning systems): `next_*` now returns a typed `IdExhaustionError` instead of panicking when a `u32` counter reaches `u32::MAX`, and every composite commit reserves its full ID budget (read-only `reserve`/`reserve_many`) before its first authoritative mutation, so a later allocation cannot strand an already-mutated owner. Boundary and save/restore-near-exhaustion tests cover the last representable allocation and the composite legal-representation commit. Audited all `next_*` consumers; no production path retains panic-on-exhaustion semantics.
