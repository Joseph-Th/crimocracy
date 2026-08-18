# Tasks

Current executable work that may be picked up asynchronously. Keep entries short; remove completed work. `STATUS.md`/`CAPABILITIES.md` owns implemented truth and `ROADMAP.md`/`DIRECTION.md` owns future direction.

## T-0001 - Add documentation integrity lane

- Area: documentation
- Next: Add a fast repository-owned documentation integrity check for the current authority set. Validate required authority documents, local links, concrete repository paths and documented local commands where mechanically decidable, and keep STATUS/README/AGENTS/TESTING roles forward-facing without duplicating semantic architecture review. Integrate the check into the documented local completion path.
- Paths: `TESTING.md`
- Verify: `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/verify.ps1 && python ../tools/check_standards.py`
- Depends: none
- Basis: `ca612fe659663a6b8e75d445ef2ecce7f9497dfb`
- Reviewed: `2026-08-18T02:48:24Z`
