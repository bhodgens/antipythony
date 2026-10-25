# Branch 06 - Module Decomposition (GATED)

Goal: split the two mega-files the review flags as size risk:
`crates/ap-run/src/builtins_modules.rs` (11,629 loc) and
`crates/ap-run/src/interp.rs` (6,590 loc). This is the ONLY branch here that
touches Plan 02 implementation files, so it carries a hard gate.

## Hard precondition (binding)

- Dispatch NO leaf in this branch while any Plan 02 leaf wave is in flight,
  and never before 02.08-full p2 (re engine) is committed AND reviewed. The
  review's own sequencing note: "schedule around 02.08-full p2, not before."
- Before dispatch: `git status` clean for the target files; read current
  PROGRESS.md ownership lines; re-measure file sizes (numbers above were the
  review's static snapshot and may have moved).

## Design rule

Split by module family along existing seams, NOT by line count:
- builtins_modules.rs -> one submodule per native module family already
  naturally separated inside it (e.g. `modules/math.rs`, `modules/json.rs`,
  `modules/random.rs`, ...). `mod.rs` keeps only the registry assembly.
- interp.rs -> executor phases: `interp/{exec.rs, eval.rs, calls.rs,
  exceptions.rs}` along its existing internal comment boundaries.
- Pure moves: NO behavior change in a decomposition commit. Every moved
  function keeps its exact body; git diff must show movement, not mutation.

## Leaves

### 06.01 - builtins_modules.rs split by module family
- Create `crates/ap-run/src/builtins_modules/` dir; move families file by
  file; `pub(crate)` re-exports preserve every call site; `mod` declarations
  in the old path replaced by the dir module.
- Gate AFTER each family move: `cargo test --workspace` green with ZERO test
  edits (tests are the contract; needing to touch one means the split moved
  behavior, not code).
- One family per commit (small, revertible). Leaf continues family by family
  across commits; agent stops when context budget nears and the PROGRESS
  note names the next family queue.
- Honest gap list: families not split.

### 06.02 - interp.rs split by executor phase
- Same rules as 06.01. Higher risk (hot path, large shared state): extract
  first the pieces with narrowest borrow-scope coupling (traceback formatting,
  call-arg helpers), leave the core eval loop last.
- If a phase boundary cannot be cut without moving behavior, STOP and record
  the blocker - a refused split is a valid outcome, a hacked one is not.
- Gate identical: full workspace tests green per move, zero test edits.

## Dispatch notes

This branch is intentionally last (Wave E). If Plan 02's remaining leaves
(02.05 tier-1, 02.06 AoT, 02.07 threading) rewrite interp.rs heavily, these
leaves re-base onto the new shape - check Plan 02 PROGRESS first, and prefer
running 06.01 before 06.02 (builtins families are the safer rehearsal).

## Test Gate

<!-- Tests-first gate (hierarchical-planning skill v1.3.0, patched 2026-10-24).
     Coverage matrix derived mechanically from leaf task headers; the control
     agent (orchestrator, in-session) refines rows and fills test names at
     Phase 0 dispatch. No implementation leaf dispatches until every row is
     VALIDATED. Delta protocol: lesser of plan/tests updates to the greater.
     Recipe: skill hierarchical-planning -> references/tests-first-gate.md -->

N/A — no leaf documents found alongside this orchestrator. If this tree
gains leaves, build the coverage matrix before dispatching them.
