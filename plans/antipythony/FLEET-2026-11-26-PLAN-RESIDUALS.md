# Fleet wave 2: plan-tree residuals (2026-11-26)

Delegation unit: deleg_ebbe3e56 (3 parallel leaves). Predecessor wave:
deleg_981aab5f (merged; worktrees deleted; issues #15/#16/#17 closed,
#18 opened).

## In flight

| leaf | branch | worktree | issue | scope |
|---|---|---|---|---|
| t3s2 | tier1-t3s2 | ~/.git/worktrees/leaf-t3s2 | #20 | T3 S2 wall: re-entrant lazy-import (pydantic BaseModel) + downstream __class__-cell chain |
| io19 | tier1-io19 | ~/.git/worktrees/leaf-io19 | #19 items 3-5 | re-verify+fix: from_bytes >255 mask, format-spec zero-fill >u128, loop default-arg capture, binascii except-leak, adaptive traceback linenos |
| ugw18 | tier1-ugw18 | ~/.git/worktrees/leaf-ugw18 | #18 | SAFETY-comment wave for all undocumented_unsafe_blocks warnings (ap-obj/parse/gc/run/ce) |

## Ownership boundaries (binding)

- t3s2: importer.rs + module-__getattr__ surfaces; stdlib_modules_differential probe (additions only).
- io19: fix_pass/import/lxml differential suites + interpreter fix files EXCEPT builtins_modules errno/select/math/importer regions.
- ugw18: SAFETY comments only (no structural edits), docs/unsafe-baseline.txt regen, docs/unsafe-inventory.md wave-2 section. ap-ce limited to targeted SAFETY-comment patches (sibling just lint-enabled it, 109 warnings surfaced).
- ALL: zero NEW unsafe (scripts/unsafe_report.sh --diff), own-crate-only fmt, no push/merge, own CARGO_TARGET_DIR.

## Known open items NOT in this wave

- #18 item: cargo-geiger CI step (needs orchestrator CI-matrix decision; deliberate defer).
- ap-cli/xtask forbid rollout: DONE by sibling commit 6880199 (issue #18 partially advanced on main already).
- epoll fromfd/EINTR/socket-fds: follow-ups named in closed #16; not scheduled.
- Inline-Python pin policing: backlog in docs/host-adaptive-pins.md; not scheduled.
- T121 gate re-baseline ADOPTION (issue #19 item 1): needs user ruling (release >=1.0x vs debug 3x).
- Plan 10.04 pydantic_core real shim + Plan 10 S2 buffer protocol: gating prerequisites for T4/T5 (issue #20 dependency order), not yet dispatched.
- conformance pytest first-ever CI leg: run 45807147027 in flight on 28480a0 (cli-docs bin fix); watcher proc pending.

## CI state at wave dispatch

- 28480a0: cli-docs gate builds ap-parse bins first (false-red fix; the committed doc had 4 NOT-BUILT placeholder sections).
- a38f490: testability-clis.md regenerated with all six bins rendered.
- 6880199 (sibling): issue #18 wave 1 — ap-ce lints on (109 warnings surfaced), forbid on ap-cli/xtask/ap-ir.

## Merge order (when unit completes)

1. io19 (small, self-contained) -> 2. ugw18 (doc/comment heavy, spans crates) -> 3. t3s2 (runtime surface, deepest verification: minimal-repro + staged-pydantic chain progress).
Each: first-hand gate re-run in its worktree before merge; fmt --all --check + stdlib differential 48/0 + unsafe_report.sh totals unchanged on trunk after the full wave.
