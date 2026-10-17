# Plan 02 progress log

## Pon study - COMPLETE (committed 1237c75)
- docs/pon-study-lowering.md (185 L) + docs/pon-study-tier1.md (208 L), clone
  d0a17177, all cited paths verified, zero verbatim copies.
- KEY ADOPTABLES (lowering): generator state-machine transform, finally
  trampoline, __classcell__, match eager-partial-bind. Parts-not-elements
  gotcha: investigated, NOT a bug in us.
- ARCHITECTURAL FORK: pon flat-CFG vs our structured ap-ir. 02.05 implementer
  decides; default = structured, documented.
- TIER-1 TOP-5 RISKS: GC roots shim / typed-OSR (net-new) / true deopt
  (highest risk, nothing in pon) / IC version-bump audit / feedback-slot
  stability. 02.05 SPLIT CANDIDATE.

## F-string bug hunt (parallel track) - COMPLETE (committed 0992ee9)
- BUG-1 debug forms + FINDING-3 walrus found; corpus slice 35+105 pinned;
  gate 107/107.

## Fix leaf (BUG-1 + FINDING-3) - COMPLETE (committed 075b453)
- debug_text through IR + dump; walrus bind_definite (+8 tests -> 72).

## Leaf 02.01 - ap-obj core values - COMPLETE (committed a5d6ab9)
- enum-of-16B Value, smi i61 + num-bigint (floor-normalized divmod),
  byte-exact hashing (int 2^61-1 / frexp float / siphash13 latin1-small-str /
  tuple xxHash / frozenset shuffle), compact dict w/ perturb+tombstones,
  key collapse, 48 tests (25 live-differential).
- Flagged: frozenset type -> 02.03; py_float_repr dedupe cleanup.

## Test leaf T-05 - COMPLETE (verified, pre-staged)
- 199 suite tests / 45 corpus cases pinned; 192 pass / 7 xfail pre-bin.

## Leaf 02.02 - tier-0 executor - LANDED, IN REVIEW-FIX (fix agent running)
- ~4300 lines, 8 modules: interp, builtins(37), exc hierarchy, PEP3101 format
  subset, frames/cells, iterator protocol, generators.
- Corpus at review time (orchestrator-verified): hello+types 2/2, fstrings
  33/35 (2 known out-of-scope), values 43/47 (V048/V050/V066 = classes 02.03;
  V016 = Rust char model vs surrogate escapes), stdlib 0/48 (02.08 surface).
  T-05: 192 pass / 6 XPASS / 1 xfail.
- ORCHESTRATOR STEER (mid-run) fixed: generator resume, iter/gen id collision
  (2-iteration cap root cause), traceback line numbers.
- FRESH-SMOKE FINDINGS (orchestrator, post-completion) -> fix agent dispatched
  (deleg_40b59065): BUG-A module-fn lookup from nested frames (self-recursion
  + mutual NameError; global VALUE reads fine, fn-name read from inner frame
  dies); BUG-B escaping closures lose cells (mk(10)(5) UnboundLocalError while
  immediate-call closure works - cell tied to maker frame lifetime?);
  BUG-C yield from (implement delegation; document return-value choice).
- Baseline for fix comparison: fstrings 33/35, values 43/47, hello 2/2.

## Leaf 02.02 - tier-0 executor - COMPLETE (committed e532638)
- All fixes verified firsthand: 24/24 differential tests green, 5/5 fresh
  smoke probes byte-match CPython (recursion, mutual, curried closure,
  shared-cell nonlocal factory, yield from). Rust 146/0, pytest 1223/0
  (s005 known), corpus baseline held: fstrings 33/35, values 43/47, hello 2/2.
- Notable root causes for the record: module-fn defs bound via store_local
  (module frames keep locals empty -> invisible); cell-classified params
  never re-seated into cells; frees not propagated through intermediate
  scopes (CPython symtable rule); nonlocal cell wiring omitted.
- KNOWN/QUEUED: debug-build stack overflow ~93 chained calls (02.05 item -
  run interpreter on worker thread or bump stack); V016 surrogate escapes
  (str-model follow-up); match class-patterns (02.03).

## QUEUE
1. 02.03 classes/protocols + T-06 - DISPATCH NEXT (unblocks V048/V050/V066,
   f12, match class-patterns).
2. 02.04 GC (MMTk) + 02.08 stdlib (unblocks stdlib slice).
3. 02.05 tier-1 (SPLIT per risks; + stack-depth item).
