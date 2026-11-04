# T-07 — gc/finalizer (gates implementation leaf 02.04 ap-gc MMTk integration)

Authorship: written from the spec pack ONLY (`docs/compat-contract.md`
C-19..C-23 + divergence register D1/D3, `docs/spec/01-values-and-objects.md`,
`docs/spec/02-control-flow.md`, `docs/spec/06-observable-behavior.md`,
`plans/antipythony/master.md`, `plans/antipythony/02-native-backend/
orchestrator.md` 02.04 Expose/Gate), per `plans/antipythony/tests/README.md`.
The author has not read any implementation source under `crates/`.

## Purpose

Gate leaf 02.04 (ap-gc: MMTk binding, Heap trait, safepoints, root scanning,
**finalizer semantics** — `__del__` runs, timing NOT guaranteed (D1) — and
**weakrefs per spec**) with an allocation-heavy differential corpus whose
every byte is pinned by CPython 3.14.7. The corpus exercises the destruction-
timing clause area C-19..C-23:

- C-19 — no observable reference counting (GC-API surface only:
  `gc.is_tracked`, `gc.get_objects` counts, `gc.collect()` return shape;
  refcount APIs are corpus-prohibited and linted out of the slice);
- C-20 — `__del__` runs at collection, not at last-reference-drop; the
  canonical interleaved program is written ONLY as the property form the
  clause's Verify line demands (gc017, gc001, gc010);
- C-21 — finalizers run at most once and eventually; stress projection with
  monotonic tokens (not `id()`) at 2000 instances (gc014), the 10k scale as
  an impl-side gate probe; unraisable `__del__` exception reported in the
  reference's `Exception ignored while calling deallocator ...` shape with
  rc 0 (gc009, pinned by RUNNING CPython 3.14.7);
- C-22 — weakref callbacks follow finalizer timing; no false collections
  (reachable survives 5 forced collects with callback unfired);
  `weakref.finalize` alive/detach/re-invoke semantics (gc004, gc005, gc008,
  gc015);
- C-23 — every case is GC-anchored: finalizer side effects go to lists and
  are sampled only after an explicit `gc.collect()`; no stdout `print` from
  any `__del__`; the suite lints this (see below).

Two halves:

1. **Fixture re-verification (green today, 113 tests).** Each `.py` under
   `tests/corpus/gc/` is re-run under the reference CPython and its
   stdout/stderr/exit bytes are diffed against recorded `expected/` files.
   Header-citation and contract-index checks, the corpus lints C-19
   (refcount-API ban), C-23 (gc.collect() anchor + no print-in-`__del__`),
   S-06-17#4 (address leak), and the 3-run byte-identical determinism check
   ride along.
2. **ap-conform gate (xfail, strict=False, 20 tests).** The 02.04 gate —
   "allocation-heavy corpus slice green; no leak across 10k iterations
   (heap growth bounded)" — expressed as `ap-conform run --corpus
   tests/corpus/gc/gc*.py --impl <cmd> --tier 0` (slice + per-case), plus
   the `sys.getrefcount`-absent impl-side projection (citing C-14 per the
   task contract; the normative text for refcount absence is C-19/D3, both
   cited in the test) and the 10k-iteration bounded-heap probe (exit 0 +
   exact-once finalizer projection + child max-RSS < 1 GiB). The reference
   side of that same probe runs as a GREEN calibration test today, so the
   1 GiB threshold is proven to be a leak detector rather than an artifact.

## Testability contract under test (from leaf 02.04's Expose/Gate)

- Expose: ap-gc surface — MMTk heap, safepoints, root scanning, finalizers
  (`__del__` runs, timing not guaranteed, cite compat contract), weakrefs.
- Gate (verbatim): "allocation-heavy corpus slice green; no leak across 10k
  iterations (heap growth bounded)."
- The only subprocess-visible seam is `ap-conform` as documented by 00.04 and
  consumed identically by T-05/T-06: `ap-conform run --corpus <glob> --impl
  <cmd> --tier 0`, comparing the three channels (S-06-1/C-27) with
  separate-stream capture (S-06-2) and scrubbed environment (S-06-15).
- Resolution order: env `AP_CONFORM`, then `ap-conform` on PATH, then
  `target/{debug,release}/`. Impl command: env `APX_IMPL`, then
  `antipythony run`. Reference: env `AP_CONFORM_REFERENCE_PYTHON`, default
  `/opt/homebrew/bin/python3.14`.
- The 10k bounded-heap gate needs an RSS measurement the corpus contract
  forbids inside a corpus case (memory numbers are non-observable, C-24); it
  is therefore a **probe** (`tests/t07_gc/probes/heap_probe_10k.py`, outside
  `tests/corpus/`) driven through `/usr/bin/time -l` by the gate test.
  Note the macOS variant of `-l` prints `peak memory footprint` rather than
  `maximum resident set size`; the test matches both spellings (live-
  verified).
- `sys.getrefcount` absence: a corpus case MUST NOT even reference the
  symbol (C-19/D3), so the ABSENT assertion lives in the impl-side xfail
  test `test_refcount_api_absent_or_stub_safe`, driving
  `probes/refcount_probe.py` under the (not-yet-existing) impl. Cites C-14
  per the task contract (the probe sits on the integer-semantics boundary
  object C-14 governs) and C-19/D3 as the normative absence rule.

## Corpus layout

```
tests/corpus/gc/            17 scripts gc001..gc017
tests/corpus/gc/expected/   gcNNN.{out,err,rc} triples (51 files)
tests/t07_gc/probes/        refcount_probe.py, heap_probe_10k.py (not corpus)
tests/t07_gc/_gen_expected.py  3-run determinism gate + expected recorder
```

Expected triples were produced by REAL runs of `/opt/homebrew/bin/python3.14`
(3.14.7, macOS arm64), each script run 3 consecutive times under the scrubbed
environment and refused recording unless all three stdout+stderr+rc tuples
were byte-identical. Result: 17/17 deterministic, zero flaps. Every case
catches what it pins in-script: rc 0 and empty stderr for all 17
(gc009's unraisable report is captured via an in-script `sys.stderr` swap,
never via the process's real stderr channel).

## Case table — filename → clauses → what is pinned → D1-timing-safety class

| File | Cites | Pins (observed 3.14.7) | D1 timing-safety |
| --- | --- | --- | --- |
| gc001_del_runs_at_collect | C-20, C-21, C-23, D1 | 3 dropped `Noisy()` → exactly 3 finalizations sampled AFTER `gc.collect()`; count stable at a second collect | SAFE: collect-anchored count; no interleave (C-20 Verify positive form) |
| gc002_finalizer_order_one_generation | C-21, C-23, D1 | one unreachable gen-0 strongly-connected set of 5 fires all finalizers before `collect()` returns, in creation order `p0..p4` | ORDER-WITHIN-COLLECT: C-23 explicitly ALLOWS "ordering after explicit gc.collect()". Order deterministic across 3 runs on reference (fixture re-verifies). If a future MMTk reordering surfaces, the *all-fired-by-return* half is the C-21 MUST; the sequence is the C-02 tie-break pin |
| gc003_refcycle_with_del | C-19, C-20, C-21, D1 | 2-node cycle + self-edge, names unbound → both `__del__`s at next collect (sorted multiset), `gc.garbage == []` (PEP 442), no double fire at a 2nd collect | SAFE: sorted multiset after collect; C-19 PEP-442 era collectability IS guaranteed |
| gc004_weakref_callback_on_collect | C-22, C-23, D1 | reachable referent survives 5 forced collects, callback NOT fired, `ref() is obj`; after last strong drop + collect: `ref() is None`, callback exactly once | SAFE: both C-22 Verify halves, all observations post-collect |
| gc005_weakref_uncollectable_targets | C-19, C-22 | weakref target matrix **pinned by running 3.14.7**: int/str/float/bytes/tuple/list/dict raise `cannot create weak reference to 'X' object`; **set IS weakref-able** (surprise caught live — spec silence); instance weakrefs clear+callback at collect for plain and cyclic instances | SAFE: TypeError presence is timing-free; callback sampled post-collect |
| gc006_gc_callbacks | C-21, C-22, C-23, D1 | `gc.callbacks` (3.13+ API): one `collect()` == `[("start",2),("stop",2)]`; `collect(0)` gen 0, `collect(1)` gen 1; info keys exactly `{collected, generation, uncollectable}`; sequence stable across consecutive collects | SAFE: callbacks fire inside the explicit collect the script itself invokes — observation point is deterministic; the *automatic*-collection emission that the raw phase list would otherwise expose is never sampled |
| gc007_is_tracked_basics | C-19, C-21 | scalars/None never tracked (False); list/dict/set/empty-containers tracked (True); tuples-with-containers tracked; plain tuples False — sampled AFTER a leading `gc.collect()` | SAFE-BY-CONSTRUCTION: CPython *temporarily* tracks freshly created tuples until the next pass proves them untracked — that pre-collect window is UNSPECIFIED for antipythony (no refcount, no gen-0 temp-track); the script therefore pins only the post-collect projection and asserts the pre-collect value merely `isinstance(..., bool)` |
| gc008_resurrection_one_round | C-21, C-22, C-23, D1 | `__del__` re-adopts self once: after collect 1, finalizer ran once and the WEAKREF still resolves to the resurrected object; after drop+collect 2, exactly one more event (the weakref callback), `count("finalize") == 1` forever | SAFE: CPython "finalize, resurrect, finalize-again-at-next-death" is the observable one-round resurrection contract; every sample post-collect; second death fires NO second finalizer (C-21 at-most-once) — pinned |
| gc009_finalizer_raises_unraisable | C-21, C-29, C-30, S-06-7, D1 | `__del__` raising during collect → 5-line stderr report captured via `sys.stderr` swap: `Exception ignored while calling deallocator <function Bad.__del__ at 0xADDR>:`, `Traceback (most recent call last):`, masked frame line, source echo `    raise ValueError("shape-check")`, `ValueError: shape-check`; `sys.unraisablehook` args = `(RuntimeError, 'boom-del', tb present, NoneType)`; rc stays 0 | SAFE with masking: address → `0xADDR` (S-06-17#4), traceback `File "<path>"` line → `FRAME` (invocation-path text is a harness concern, spec 02 convention). The hook's `object` field is `NoneType` on 3.14.7 for deallocators — pinned as observed, not assumed (C-29 "measured not assumed") |
| gc010_del_alias_does_not_fire | C-19, C-20, C-23, D1 | `del a` while `b` aliases the same object → mid-run count 0 (BOTH models agree: refcount ≠ 0; GC: reachable), still 0 after collect-while-aliased, exactly 1 after final drop + collect | SAFE BY DESIGN (task requirement): assert-on-total-not-order; the `del keeper` line is where CPython fires and antipythony MAY defer — nothing is sampled between `del keeper` and the following `gc.collect()`, so prompt and deferred destruction produce identical bytes |
| gc011_collect_returns_and_garbage | C-19, C-21, D1 | 10-node closed cycle, all names unbound → `found >= 10` True; callback `uncollectable` == [0]; `gc.garbage == []` | SAFE-with-UNSPECIFIED-carve-out: the raw `collect()` integer (observed 124) mixes in interpreter-startup garbage — stable across 3 runs here and across path lengths checked, but patch/build sensitive → pinned as a lower-bound boolean, raw count left UNSPECIFIED (see Non-determinism log) |
| gc012_gc_disable_explicit_collect | C-20, C-21, C-23, D1 | `gc.disable()` → `isenabled()` False; explicit `collect()` STILL finalizes the dropped object (`['d']`); remains disabled; `enable()` restores; no double fire | SAFE: C-20's forced-finalization is about collect(), not scheduling; counter sampled post-collect only |
| gc013_get_objects_filtered | C-19, C-21, D1 | `gc.get_objects()` instance count: 4 while held → 0 after drop+collect → 0 after self-cycle+collect — no `repr`, no `id`, no addresses anywhere | SAFE: count projection only; reachability, not timing, decides membership at the sampled instant |
| gc014_finalizers_once_stress | C-21, D1 | 2000 finalized instances + interleaved cyclic garbage, one forced collect → `fired: 2000`, `unique tokens: 2000`, `tokens complete: True` (token-keyed NOT id-keyed per C-21 moving-heap rule), no second-round fire | SAFE: C-21's exact Verify shape at corpus scale; the 10k×10-round scale lives in the gate probe (C-24 forbids RSS in corpus cases) |
| gc015_weakref_finalize_api | C-21, C-22, C-23, D1 | `weakref.finalize`: fires at collect (not before), `.alive` flips, re-invoke returns None, `detach()` → 4-tuple with `(obj,func,args,kwargs)`, invoking detached runs the callback manually and no at-exit callback fires for it | SAFE: all samples post-collect; the pending atexit-path finalize is printed BEFORE process exit, never after — its at-exit firing is timing (D1) and is deliberately not asserted (C-22 guarantees the atexit path exists; the gate slice can't observe its instant) |
| gc016_threshold_and_counts | C-19, C-21, D1 | threshold round-trip; `set_threshold` returns None; `get_count()` shape (3 ints); gen0 == 0 after collect; **`gc.freeze()` and `gc.unfreeze()` return None on 3.14.7** (docs say "number of objects" — caught live: it does not); frozen name-dropped object: FINALIZED exactly once and GONE from `get_objects()` after unfreeze+collect | SAFE-end-state-only: whether the frozen object dies at the first collect (CPython: refcount cascade through its gen-0 list member does) or only after unfreeze (a tracing GC has no gen-0 temp-track story) is a D1 model difference → intermediate states NOT sampled; only the after-collect end projection pinned (see Non-determinism log; the FIRST naive attempt pinned "frozen survives collect: True" and was caught flapping against reality by the live probe) |
| gc017_heartbeat_property_form | C-20, C-23, D1 | the C-20 canonical program as the property form its Verify line demands: `make\nmake\nmake` (timing-free — `__init__` prints), then collect-anchored `dels: 3 total: 3`, then `loop done` | SAFE: byte-interleaving of `del` lines vs the loop is EXACTLY the D1-divergent observation; finalizer writes to a list, and the deterministic-after-collect sequence `make×3, del×3(count), loop done` is what is pinned |

## D1-timing-safety analysis (summary)

Rule applied to every case (C-23): an assertion is legal only if its
observation point is an explicit `gc.collect()` the script itself calls, or
a value no destruction model can see (TypeError presence, API return
shapes, counts of reachable objects). The suite enforces this mechanically:
`test_gc_cases_cite_the_divergence_when_timing_sensitive` fails any case
whose `__del__` prints to stdout, or whose timing-related citation lacks a
`gc.collect()`; `test_no_refcount_api_in_corpus` implements the C-19 lint
at the fixture level while the harness-side load-time rejection is an open
gap (G-2); `test_determinism_three_identical_runs` implements C-23's
"reject a case that flaps" for a budgeted subset (all 17 were 3-run
verified at recording time by `_gen_expected.py`).

Cases whose output would differ under CPython-without-collect vs
CPython-with-collect were rejected at authoring; cases whose output differs
between refcount-prompt and GC-delayed models were rewritten to the
total-not-order form (gc010) or end-state-only form (gc016) rather than
dropped.

## Non-determinism log (what resisted pinning, and the projection kept)

1. **Fresh-tuple temporary tracking (gc007).** CPython tracks a newly
   created plain tuple for one GC pass before untracking it; the pre-
   collect value is timing-sensitive and has no tracing-GC analogue.
   Handled: leading `gc.collect()` before every `is_tracked` sample; the
   pre-collect value is asserted only to be a bool. UNSPECIFIED: the
   temporary-tracking window itself.
2. **`gc.collect()` raw return (gc011).** Observed 124 — includes
   interpreter-startup garbage; identical across the 3 recording runs and
   across invocation path lengths (checked), but a patch-level/build-level
   coincidence, not a contract value. Handled: pinned as `found >= 10`
   (the cycle's own nodes) — the stable projection. UNSPECIFIED: the exact
   reclaimed count.
3. **Frozen-object intermediate lifetime (gc016).** CPython frees a
   frozen, name-dropped object during the FIRST collect via the refcount
   cascade of its mutable member (freeze moves GC-visibility, not Python
   references); a tracing GC has no such cascade and may finalize only at
   unfreeze+collect. Handled: pin only the end state (exactly-once
   finalization by the second collect + gone from `get_objects()`) and the
   API return values (None/None, verified live). UNSPECIFIED: which
   collect fires the finalizer.
4. **`gc.callbacks` on automatic collections.** Automatic gen-0 collections
   inside the loop would emit extra ("start",0)/("stop",0) events at
   nondeterministic script positions. Handled: hooks are registered only
   around script-owned `gc.collect()` calls and cleared immediately; the
   pinned sequences contain only explicit-collect events. UNSPECIFIED:
   when/whether automatic collections happen at all (pure D1).
5. **Finalizer order within one collect (gc002).** All-five-before-return
   is a C-21 MUST; the creation-order sequence is deterministic on the
   3.14.7 reference (3-run verified) and therefore pinned by C-02 tie-
   break. Flagged for 02.04 review: if MMTk's finalize set provably orders
   differently, this is the ONLY corpus sequence that would need a contract
   amendment — order-within-collect is not covered by D1's timing-only
   carve-out as written, so any divergence there MUST go through §9 by PR
   (C-02), not through silent relaxation.

## CONTRACT-GAPS

- **G-1 — gc.get_objects / gc.callbacks / gc.freeze / gc.is_tracked are not
  in the v1 stdlib floor (C-05).** The 22-module floor does not name `gc`;
  spec 05's list governs, but T-07's entire observation surface assumes the
  `gc` module is native-provided. If spec 05 does not list it, this slice
  cannot gate 02.04 through ap-conform. → back to contract/spec-05.
- **G-2 — the C-19 corpus lint + harness load-time rejection has no
  executable home yet.** C-19 Verify names a lint over `tests/corpus/` and
  a harness that rejects refcount-calling cases at load. Neither the lint
  tool nor the ap-conform load hook is documented by 00.04; the fixture
  test here is a stand-in until the harness ships.
- **G-3 — C-23 forbids generator-close style "cleanup ran at a specific
  point" but 02.04's expose text covers finalizers only.** Generator close
  timing under MMTk (S-02-25 `g.close()` is explicit and deterministic;
  *implicit* close at GC is timing) is not addressed by any clause; T-07
  excludes generator-GC cases. Flag: whoever gates implicit generator
  reclamation needs a D1-extension ruling.
- **G-4 — `weakref.finalize` atexit-path firing is unobservable within a
  byte-diff contract.** C-22 says the atexit path "still runs"; its firing
  happens after all possible script sampling. gc015 pins everything
  observable and leaves the at-exit firing to a property the harness cannot
  diff (would need an ordering probe across process exit, out of v1
  channels). No case weakened: firing was simply never asserted.
- **G-5 — freeze()/unfreeze() return values.** CPython docs claim a count;
  3.14.7 returns None for both (pinned live). Under C-01/C-02 the *measured*
  reference value governs, but spec 05's module text, if it mirrors the
  docs, will contradict the corpus. → verify against the 3.14.0 pin when
  C-01's re-verification item closes.
- **G-6 — heap-bounded gate has no numeric contract.** 02.04 says "heap
  growth bounded"; no clause fixes a threshold or an RSS channel (C-24
  makes memory non-observable for conformance). The gate test uses a
  generous 1 GiB ceiling on the 10k×10-round probe, calibrated green on
  the reference (observed peak footprint ~8 MB). The number is
  test-harness policy, not a conformance contract; if 02.04 wants it
  contractual it needs a new clause.
- **G-7 — reference build drift (inherited).** Expected bytes recorded on
  3.14.7; C-01 pins 3.14.0. All 17 cases are provisional until re-recorded
  on the pinned build — same open item as T-05/T-06 and the contract
  PROGRESS notes. Re-recording procedure: `python3.14
  tests/t07_gc/_gen_expected.py` refuses to write on any 3-run flap.
- **C-14 citation note (task-contract wording).** The getrefcount-absent
  xfail test cites C-14 as instructed; C-14's own text is int
  overflow/memory exceptions, and the refcount-absence rule lives in
  C-19/D3. The test cites both and states the mapping so a reader is not
  misled. If this pairing was intended to mean something else, it is a
  contract-editorial item, not a semantics gap.

## Results (at authoring)

`/opt/homebrew/bin/python3.14 -m pytest tests/t07_gc/ -q` →
**113 passed, 20 xfailed** (zero failures, zero collection errors, ~1.2s).
Corpus: 17 cases (≥15 floor met), 51 expected files, all 3-run deterministic
byte-exact on CPython 3.14.7.
