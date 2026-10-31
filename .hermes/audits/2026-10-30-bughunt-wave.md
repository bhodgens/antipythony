# antipythany bughunt wave 2026-10-30 — window c51d78d..HEAD (255 commits)

Method: 4 read-only auditors (wave-7 TALLY closure audit; interp/frame/
generators/threading; builtins/classes/methods/importer/exc_hier/ops;
gate ecosystem xtask+CI+fuzz seeds) + parent verification.

CONCURRENCY NOTE: a sibling session held the tree the whole wave (61 dirty
files incl. interp.rs, threading.rs, generators.rs; waves 105-107 landed
mid-audit). No fixer was dispatched. This report is audit-only for this
repo; the sibling owns the tree.

## Parent-verified findings

1. [CRITICAL-at-HEAD, ALREADY-FIXED-BY-SIBLING] threading.rs:99-100 —
   `GIL.mu.lock().unwrap()` + `cv.wait().unwrap()`: a poisoned GIL mutex
   panics every subsequent acquire. Parent verified the unwrap pair still
   exists at HEAD (committed). Sibling's dirty diff does not touch it.
   Real hazard: one panic under threads poisons the process. Fix shape:
   recover from poison (`PoisonError::into_inner`) — the GIL invariant
   survives a panicking holder.
2. [HIGH] generators.rs:405 `expect("generator fiber stack")` — panic on
   stack allocation failure instead of MemoryError.
3. [MEDIUM] generators.rs:197-221 — spin loops with try_borrow_mut under
   contention (busy-wait, no backoff).
4. [MEDIUM] wave-7 TALLY closure: 6 FIXED, 5 PARTIAL, 20 STILL OPEN
   (def-time defaults reverted intentionally; StopIteration.value,
   finally-return, itertools.count start/step, accumulate initial=,
   lru_cache.cache_info, nested partial, range-in-bigint membership,
   4 missing SyntaxError classes...). These are tracked gaps, not new
   bugs — the repo's wave process owns them.
5. [MEDIUM] crash-fuzz seeds: list-multiply capacity panic (fixed 97cc2fc)
   has NO regression seed in plans/robustness-ci/04-fuzz/FUZZ_SEEDS.txt.
6. [PASS] stdlib shims (secrets/ftplib/bz2/lzma), dict(mapping) protocol,
   class-body exec CURRENT_CLASS_NS, ops.rs binop fixes — all verified
   correct with pinned differential corpus.
7. [PASS] attribution.yml deletion: gate migrated to fast-lints, verified.
8. [PASS] xtask stdlib-provenance + corpus_traceability checks: no bypass.

## Parent probe (real binary, target/debug/antipythany)
- `[1]*10**9` -> MemoryError (no panic) — 97cc2fc fix live.
- for/else break-suppression: fixed (c8eaf61) — probe ran clean.

## DISCLOSURE LEDGER (auditor noise)
- ant-a2 #8 "exception state corruption": INFO, not reproduced.
- ant-a4 NEXT-STEPS 412-vs-396 test count: methodology difference
  (tests/ dir), not a bug.
- ant-a1 several "needs verification" rows: parent spot-checked the two
  crash-fuzz rows with the real binary (both fixed).

## Fix disposition
- NO fixer dispatched: sibling session owns the dirty tree; editing
  interp/threading/generators now would collide. Findings 1, 2, 3, 5 are
  queued for the next clean-tree wave (file:line pinned above).

## SECOND-WAVE ADDENDUM (b1 frontend, b2 GC/object)

- b2 #1 [HIGH claimed] "frame locals not registered as roots": REFUTED
  by parent. weakbridge_frame_values() returning Vec::new() is
  deliberate (gcsupport.rs comment): the subtract-refs seeding in
  mark_reachable (gcmod.rs: sc > internal+1) covers native-stack locals
  by construction; module namespaces are explicit roots.
- b2 #2 [MEDIUM] nested CollectGuard clears the flag at inner drop -
  VERIFIED in code, documented as MMTk-swap debt. Queued.
- b2 #4 [MEDIUM] SEQ/SEQ_OF finalizer-order staleness across
  collections: queued for the clean-tree wave.
- b1 #2/#3 [MEDIUM] lowering.rs expect("scope stack never empty") at
  255/306/468 + 1161 contradict the file's "no panic paths" claim if the
  invariant can break; UNVERIFIED (no PoC). Queued with a fuzz-seed ask.
  Binary probes: malformed input yields SyntaxError, not panics.
- b2 #7 [MEDIUM claimed] "edges_of skips class_ref/type_ref": parent
  verdict = direction of error is CONSERVATIVE here. mark_reachable
  seeds any entry with sc > internal+1 as externally referenced; an
  untraced instance->class Rc makes internal_edges UNDERCOUNT, so the
  class is seeded and RETAINED (possible leak), never collected early.
  Not a soundness bug; queued as a leak observation only.
- b2 #5 [LOW] INTERNED_TYPE_PINS unbounded: accepted (immortal types,
  bounded by program type count).
- b1 #9 [INFO] window-diff regression review: no panic-inducing changes
  in ap-parse/ap-ir/ap-cli.
- Sibling still active (wave-108 landed mid-audit; 50 dirty files).
  Disposition unchanged: audit-only, pinned for next clean wave.
