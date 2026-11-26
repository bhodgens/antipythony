# for-loop-report — leaf hp-forloop (03-jit-for-loops/01)

Branch: tier1-hp-forloop. Commits: 36a20fa (desugar + seams + backward
liveness), 839befd (OSR header fix + 3 gate-found JIT-boundary bugs).

## What landed

1. **Stmt::For lowering** (cfg.rs): boxed iterator protocol desugaring —
   `it = IterNew(iterable)` once; head block = `OsrPoll + IterNext +
   Branch(exhausted → else / body)`. The StopIteration exit edge is a
   plain Branch on an exhausted-flag temp (CPython FOR_ITER shape),
   NOT an exception park — a user `raise StopIteration` inside the
   body still routes through the exception machinery.
2. **Break/Continue/else**: open-loop context stack; while gained
   break/continue/else too; else-on-normal-exclusion via the brk
   landing block. Out-of-loop break fails closed (`Unsupported("Break
   outside loop")`).
3. **IterNew/IterNext host seams** (runtime.rs + jit_hook.rs): the
   ap-run side routes through `register_iter_pub`/`pull_iter_pub` — the
   EXACT tree-walker Stmt::For surfaces (generator-aware pulls, live
   table handles, user `__iter__`/`__next__`). `h_iter_next` writes the
   value + exhausted flag and returns 0 for both produced and
   exhausted; only real errors park.
4. **Backward-dataflow OSR liveness** (osr.rs `live_in_at`): true
   use/def fixed point with PRECISE reads/writes (the old shared
   `reads_of` counted inst DESTINATIONS as reads — every read-anywhere
   temp inflated the live set). For-loop live sets (iterator handle +
   exhausted-flag temps) now stay under OSR_MAX_LIVE.
5. **List/Tuple/Dict literals** in `lower_expr` (boxed protocol: 
   list()+append / tuple(list) / dict()+StoreSubscript) — the bench
   shapes iterate literals.
6. **Gate-found fixes** (all repro'd + verified on the way):
   - `loop_headers` resolved the for-shape OSR header to the PRE-HEAD
     (holding IterNew) — an OSR transfer re-ran iter() with a fresh
     iterator and the OSR body re-processed consumed items
     (_ast_unparse doubled one subtree per transfer). Fix: header =
     the poll block itself (both desugars now place the poll at the
     head), validated on a real back-edge cycle. APX_OSR_OFF=1 debug
     knob added for A/B isolation.
   - JIT'd raise boundary: h_raise parks the exception INSTANCE in
     ctx.ret; jit_call_body read pending_exc (None) → every propagated
     JIT'd raise became SystemError. New `CallOutcome` enum; Raised
     converts through exception_value_of_instance_pub (AoT's surface).
   - Metaclass classmethod read on the class leaked the raw descriptor
     → "'object' object is not callable" (abc.ABCMeta.__subclasshook__);
     now unwraps + binds cls. Pre-existing main-line bug (tier-0 repro).
   - JIT'd CMP_IS/CMP_NOTEQ lacked the NotImplemented name-singleton
     arm → `ok is not NotImplemented` inverted in JIT'd bodies.

## Targets

- **5 iterator kinds COMPILE** (list/tuple/str/range/dict), zero
  lower-rejects, zero fallbacks — probe5 (all kinds + nested +
  break/continue/else) byte-identical vs CPython 3.14 under tier-0 AND
  adaptive.
- **int-accum for-loop: 2.16x** adaptive vs tier-0 (min-of-7 wall
  clock; target >= 1.8x). OSR transfers=40 on the 40-call probe.
- StopIteration/break/continue/else byte-identical (probe5 +
  while_regress incl. while-else/while-True-break).

## Gates (all green, final tree)

- ap-conform stdlib **48/48 adaptive AND tier-0** (s023 AND s031
  previously failing adaptive — both fixed); values **50/50** both.
- 7 diff gates: diff_b64, diff_hex, fix_int (9508 lines), fix_json,
  fix_sort, fix_random, fix_sha_zlib — all byte-identical vs CPython.
- cargo test -p ap-codegen-native --release: 70 passed / 0 failed.
- cargo test -p ap-run --release: 208 passed / 0 failed / 1 ignored.
- tier_mode_differential 3/3; osr_gate 2/2.
- 17 tier1-transparency probes: mode-pair + CPython byte-identical.

## Honest gaps

- **Tier-1 typed regions do not form for for-loop bodies** whose block
  0 holds IterNew (block-0 rooting per the 02.05c soundness rule) —
  for-loops run the compiled JIT baseline + OSR (2.16x), not the
  typed fast path. Widening the region finder to for-loop bodies is
  the follow-up (the body blocks between head and exit ARE eligible;
  only the block-0 rooting blocks them).
- **While-loop poll moved** from body to cond block (part of the OSR
  header fix) — while OSR behavior re-verified via osr_gate + probe5 +
  while_regress, but per-shape while benchmarks were not re-measured.
- fix_int / fix_random probes are slow (minutes) under this tree's
  debug-knob builds; verified byte-identical once each, not re-run
  per-commit.
