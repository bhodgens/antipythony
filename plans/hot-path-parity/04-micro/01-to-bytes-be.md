# 01-to-bytes-be — micro tails: BE single-pass emit + str.split fast path

## DISPATCH INSTRUCTION

Worktree /Users/caimlas/.git/worktrees/leaf-hp-micro (branch
tier1-hp-micro, from current main). Own commits, explicit pathspecs.
ONE heavy cargo lane; CARGO_BUILD_JOBS=4;
CARGO_TARGET_DIR=$PWD/target-hp-micro everywhere.

Parent: plans/hot-path-parity/master.md. SMALL filler leaf (~30K) —
dispatch to any idle lane. NOTE: pp2-int (in flight) already owns the
to_bytes Smi/LE bulk-emit rewrite; CHECK ITS MERGED STATE FIRST — if
merged, this leaf does the BE single-pass (below); if not merged,
only do the str.split item and leave to_bytes alone.

## Items

1. **int.to_bytes BE single-pass** (only if pp2-int merged): the LE
   path is one bulk memcpy of the u32 limb array; the BE path still
   memcpy-then-reverse (two passes over 1MiB). Emit BE directly:
   iterate limbs last-to-first, swap_bytes each u32, copy_from_slice.
   One pass, no reverse. Target: BE within 10% of LE.
2. **str.split fast path**: the audit rows show 0.47-0.51x; the cost
   is per-piece Value/Gc allocation. If the GC nursery leaf has
   landed, re-measure first (split may already be fixed by cheaper
   allocation). If still slow: split-on-ASCII-space and
   split-on-single-char via memchr-style scan producing offsets,
   then batch-mint pieces; multi-char separator via the bytes arms'
   search. Target: within 25% of CPython.
3. **str.join homogeneous arm**: if pp2-memchr has landed, re-measure
   join; else implement the borrowed list-of-str fast path (sum
   lengths, single alloc, bulk copies) — audit says the generic path
   is the cost.

## Gates (each item, before perf claims)

7 diff gates + ap-conform values 50/50 + stdlib 48/48 + cargo test
-p ap-run --release. New differential cases for BE to_bytes (every
length 0..64 both endiannesses) and str.split/join (separator
classes, empty pieces, maxsplit) go into fix_int.py / fix_sort.py's
style — a new fix_micro.py gate file if cleaner.

## Report

plans/hot-path-parity/04-micro/report.md — per-item before/after.

## Self-verification checklist
- [ ] BE within 10% of LE (or documented why)
- [ ] str.split within 25% of CPython (or nursery already fixed it —
      re-measured and documented)
- [ ] all gates green
