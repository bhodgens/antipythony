# NEXT-STEPS: GC/memory campaign pause (2026-11-05 evening)

User ordered stop after a memory spike. State below lets the next session
resume without re-deriving anything.

## What landed (committed)
- `dea6c03` — test harness: subprocess-per-probe executor (`ap-run-probe`
  bin via `CARGO_BIN_EXE_`), cross-process flock
  (`/tmp/aprun_heavy_gate.lock`), per-probe fence (6 GB RSS / 240s
  deadline, `AP_PROBE_MAX_RSS_GB` / `AP_PROBE_TIMEOUT_SECS`). All verified
  live. 7 ap-run differential suites converted. docs/development.md has
  the memory-discipline section.

## What is IN THE WORKTREE, UNCOMMITTED, all tests green
- `crates/ap-gc/src/gcmod.rs` — GENERATIONAL gen-0 collection: automatic
  threshold fires scan only the YOUNG set (objects registered since last
  gen-0 pass); explicit `gc.collect()` still scans everything. Removes the
  O(n²) full-heap-scan-per-700-allocations behavior. All 23 ap-gc unit
  tests + invariants green.
- `crates/ap-parse/src/lowering.rs` — one-shot line-start index +
  binary-search `line_at` (was: rescan-from-byte-0 per AST node, O(file²)
  per module). ap-parse tests green.
- VERIFIED effect: 3M-object list-append probe went from "never finishes,
  44 GB" to 89s / 60 MB.
- DO NOT commit together with sibling WIP files (crates/ap-run/src/*).
  Commit by explicit pathspec (`crates/ap-gc/src/gcmod.rs
  crates/ap-parse/src/lowering.rs`) after re-running `cargo test -p ap-gc
  -p ap-parse`.

## CORRECTED diagnosis (HEAD baseline 5aca923, worktree-measured)
- RESOLVED 18:10: gcmod.rs + lowering.rs committed as bcc0a04 (cascade +
  scoped scans + line index). List probe: 101s/60MB VERIFIED on the
  commit. All 23 ap-gc + 7 ap-parse tests green.
- Unittest-import probe: STILL GROWS with every GC policy (gen-0 only:
  11GB@30s; cascade: 32.8GB@246s) but PASSES at HEAD 5aca923 (7.3s/67MB).
  => NOT a GC bug. The leak is produced by the uncommitted
  crates/ap-run/src WIP interacting with import-heavy paths. Owned by the
  sibling leaf's commit gate. Do NOT tune the GC against it further.
- The sibling session (20261104_224249, W111/W113 crawl) repeatedly
  launches UNBOUNDED probes of this leak (t1gate-bin, mem-bin — the
  latter with no timeout wrapper, 22.5GB at user-ordered kill).
  Orchestrator killed three such runs this session at user direction.

## The spike the user saw
Orchestrator-run UNFENCED probes (`target/debug/ap-run-probe` launched
bare in background sampler jobs) on the sibling's leaky unittest path.
STANDING RULE: manual probes go through a fenced wrapper, never bare.

## Resume queue
1. When the sibling leaf commits: re-run fenced
   `AP_PROBE_MAX_RSS_GB=6 AP_PROBE_TIMEOUT_SECS=240 cargo test -p ap-run
   --test gc_differential --test metaclass_arity_differential`.
2. Commit gcmod.rs + lowering.rs by explicit pathspec (gates above green).
3. Re-measure /tmp/probe_unittest.py and /tmp/probe_rss.py on the merged
   tree; if unittest still leaks on the MERGED tree, revisit the cascade
   hypothesis with fresh profiles.
4. Clean up: `git worktree remove /tmp/ap-bisect` (bisect worktree, job
   done).
