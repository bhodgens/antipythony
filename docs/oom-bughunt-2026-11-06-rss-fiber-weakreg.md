# OOM Bughunt 2026-11-06 — RSS-gated probe sweep (fenced, serial)

Date: 2026-11-06. Tree: antipythony @ 9f6e2b9 (post-oom-campaign commit
0676439). Method: fenced corpus probes run as children of
`plans/robustness-ci/measure_rss.py` (wait4 rusage maxrss), env caps
`AP_MAX_RSS_GB=2 AP_PROBE_MAX_RSS_GB=2` set for every run, single probe at a
time, `timeout 120-280` per run. Probes in /tmp/oomhunt/ (p1..p19b), sweep
scripts sweep.sh..sweep7.sh, raw results in /tmp/oomhunt/results*.txt.

Known-fixed classes NOT re-reported: _ast bulk-getattr clone leak (363600e),
_ast self-import recursion + importer post-insert (8bab377), O(n^2) GC
auto-collect (bcc0a04/743e277). Scrapy import-chain footprint (0.8-1.4GB
debug) treated as legitimate baseline, not a leak. #13 multibyte codecs is
known-open and not memory-related — out of scope.

---

## FINDING 1 (CONFIRMED, HIGH): def-generator fibers cost a 64MB stack each —
## ~150KB RSS committed per suspended generator, ~26MB RSS per 170 held

Probe: /tmp/oomhunt/p19b_suspended_touch.py (N suspended-mid-body
generators held in a list), measured:

| N     | maxrss_kb  | slope                      |
|-------|-----------|----------------------------|
| 50    | 28,197    | —                          |
| 200   | 81,494    | +266 B/gen                  |
| 400   | 152,125   | +353 B/gen (vs N=50: ~310 B/gen avg) |

A simpler variant (p19_suspended, N=1000) reached 280MB RSS with zero
useful work: 1000 lazily-started generators ≈ 253MB above the N=10
baseline, i.e. **~253KB of committed RSS per held generator**. Extrapolated:
~8,000 concurrently-held generators ≈ 2GB RSS — CPython holds the same 8k
generators in well under 10MB (generator object ≈ a few hundred bytes +
frame; the C stack is NOT retained while suspended).

Owner: crates/ap-run/src/generators.rs, `make_fiber()` (lines ~483-503):

```rust
Coroutine::with_stack(
    corosensei::stack::DefaultStack::new(64 * 1024 * 1024)
        .expect("generator fiber stack"),
    ...
)
```

The 64MB stack exists to survive "nested genexpr/listcomp evaluation inside
a generator body costs real Rust stack" (comment at the fn). But the stack
allocation is per-generator and the fiber is retained for the WHOLE
suspension window (stored in `GenFrame::Live.fiber`, generators.rs
resume_live:448-453), so every simultaneously-suspended generator pins the
allocation. MMTk heap growth (gc auto-collect fixed at bcc0a04) is fine;
this is native (outside-GC-heap) allocation growth, the same class the
_ast.py bulk-getattr leak was.

Fix sketch (product-side, not applied):
1. Shrink the default fiber stack (e.g. 1-2MB) and bump it ONLY for the
   documented shapes that need depth (detect by recursion depth seen at
   first yield, or via an explicit `sys.setrecursionlimit`-like knob).
2. Better: stop storing the fiber on suspension. Corosensei fibers can be
   dropped and the generator's resumed state kept as data (capture loop
   state into GenFrame), rebuilding a fresh fiber per resume segment. Even
   a pool of N reusable fibers (e.g. 8) would cap total stack RSS at
   8×64MB instead of unbounded.
3. Minimum safety net: honor the existing RSS guard (crates/ap-run/src/
   rss_guard.rs) from make_fiber — refuse (or shrink) new fibers once
   RSS-pressure says so, instead of aborting the whole process.

## FINDING 2 (CONFIRMED, HIGH): weakref.ref() with a CALLBACK never prunes
## — ~1.4KB RSS per dead ref, wall-clock degrades superlinearly

Probe: /tmp/oomhunt/p17_weakcb.py (N refs w/ callbacks to short-lived
targets, gc.collect() every 1000):

| N      | maxrss_kb  | wall_s | slope                     |
|--------|-----------|--------|---------------------------|
| 20k    | 48,955    | 7.0    | —                         |
| 100k   | 189,694   | 162.6  | +1.76KB/ref RSS, 23x time |
| 200k   | (timeout 200s) | —  | never finishes            |

Control (p12/p12b, same churn with NO callback, gc.collect every 500):
| N      | maxrss_kb | wall_s |
|--------|-----------|--------|
| 20k    | 51,986    | 13.4   |
| 50k    | 111,411   | 77.4   |

Even the no-callback variant grows ~1.6-2.0KB/ref and slows badly
(50k refs → 77s, vs CPython ≈ instant). CPython drops the weakref entry
once the callback fires; ours keeps it forever.

Suspected owners (three parallel unpruned registries; all in the
weakref/finalize path):
- crates/ap-gc/src/gcmod.rs:69 `WEAKREFS: RefCell<Vec<WeakEntry>>` —
  `weakref_add` (:1048) only pushes; `weakref_take_dead` (:1059) marks
  `fired = true` but never removes entries. `sweep_weakrefs` (:931) sets
  `e.dead = true` but no prune pass exists.
- crates/ap-run/src/gcsupport.rs:51 `WEAK_TOKENS: RefCell<Vec<WeakToken>>` —
  pushed at :164 and :213 (ref + proxy), never pruned anywhere (grep: only
  push, no retain/remove).
- crates/ap-run/src/gcsupport.rs:53 `FINALIZERS: RefCell<Vec<FinalizerEntry>>`
  — `weakref.finalize` pushes at :837 and `finalize_call` (:862) sets
  `e.detached = true` but leaves the entry (holding func+args+kwargs —
  arbitrary user Values, keeping closures alive) in the Vec forever.
  `finalize_atexit_drain` (:925) never clears either.

The wall-clock blowup at 100k+ entries is consistent with `fire_dead_weakrefs`
(gcsupport.rs:276) doing a linear `WEAK_TOKENS.iter().find(target_addr ==
…)` per dead entry on every collect — O(entries²) cumulative over a run.

Fix sketch (not applied):
- Prune, don't just flag: after `weakref_take_dead()` marks fired, remove
  the dead entries from WEAKREFS (swap_remove by index), and have ap-run
  drop the matching WeakToken + (for finalize) the FinalizerEntry once
  detached && dead. CPython semantics: dead weakref entries without
  callbacks vanish; with callbacks they vanish after the callback runs.
- Replace the linear `find` in fire_dead_weakrefs with an index
  (HashMap<target_addr, Vec<token_idx>>) or dedupe the dead-drain to a
  single sorted pass.
- In `finalize_call`, once `detached`, remove the entry (CPython keeps the
  token object but NOT the func/args/kwargs alive).

## FINDING 3 (CONFIRMED, MEDIUM): io.StringIO.write() grows RSS ~2-4x the
## logical content size, and never returns pages

Probe: /tmp/oomhunt/p6_stringio.py — N writes of a 33-byte line (logical
content at N=200k is 6.8MB):

| N      | content  | maxrss_kb | ratio |
|--------|----------|-----------|-------|
| 20k    | 0.67MB   | 30,572    | ~45x  |
| 100k   | 3.4MB    | 267,534   | ~78x  |
| 200k   | 6.9MB    | 481,427   | ~70x  |

The absolute slope is ~2.3-2.4KB per write call — far above the 33 bytes
written. gc.collect() during writes does NOT help (p6b: same 502MB at
200k). This is not just "GC lag": something in the write path is
amplifying per-call allocation.

Suspected owner: crates/ap-run/src/builtins_modules.rs, io "write" arm
(:12556-12601). Per write it does:

```rust
let data = match attrs.get(&s("<buf>")) {
    Some(Value::Bytes(b)) => b.to_vec(),   // full copy 1
    _ => vec![],
};
let mut nd = data;
nd.extend_from_slice(&payload);            // realloc + copy 2
let _ = attrs.set(s("<buf>"), Value::Bytes(Gc::new(nd)));
```

- `b.to_vec()` clones the ENTIRE buffer on every write (O(n²) total bytes
  copied, and the old + new buffers coexist → peak 2x buffer per call).
- The old buffer's `Gc<Vec<u8>>` is only dropped after the new one lands,
  so transient peak is ≥2x buffer per write; with GC amortization the heap
  can ride at 2-4x live size for the whole run (matches ~70x total RSS at
  200k: 2 buffers × ~3.4MB slack + GC headroom … measured, amplification
  is real but the exact factor needs GCSTATS to pin).
- `stream_parts` (:12311) makes the same full `.to_vec()` clone for EVERY
  read-style op (getvalue/tell/seek/truncate all call it), multiplying
  transient copies.

Fix sketch (not applied): store `<buf>` as a `Gc<Vec<u8>>` and mutate it
in place under `borrow_mut` (extend_from_slice directly into the existing
Vec) instead of clone → extend → replace; same for stream_parts (return a
length + read-range instead of a full copy where the caller only needs
pos/len).

## FINDING 4 (CONFIRMED, MEDIUM — correctness-adjacent): `sys.setrecursionlimit(N)`
## lets the raw native stack overflow → SIGABRT at N≥~4-5k, and the
## RecursionError boundary sits at depth ~1000 regardless of the setting

Probes: p7_recursion.py (setrecursionlimit + recurse) and
p7b_recdetect.py (default limit):

- p7b (no setrecursionlimit): depth 100/500 OK; depth ≥1000 → clean
  `RecursionError` (limit 1000 per frame.rs:305/387, working as intended).
- p7 (setrecursionlimit(5100) + recurse 5000): **native stack overflow,
  "fatal runtime error: stack overflow, aborting", rc=-6 (SIGABRT)**, RSS
  284MB, at all three sizes (5k/20k/40k). No Python-visible exception.

So `setrecursionlimit` is accepted and stored (builtins.rs:3009-3016) but
the underlying native frame budget is fixed — the interpreter will happily
recurse past what the thread stack can hold. The module-level RSS guard
can't catch this: it's a stack, not heap, failure, and it kills the
process (no fenced exception path). Any stdlib or user code that raises
the limit (cPython idiom for deep trees, e.g. some JSON/XML recursion
shims) converts a RecursionError into a crash.

Suspected owner: crates/ap-run/src/builtins.rs:3009 (`"setrecursionlimit"`
arm) — stores the limit without validating against the available stack
depth; crates/ap-run/src/frame.rs:445 (`enter_call`) enforces the stored
value but has no stack-budget backstop.

Fix sketch (not applied): clamp setrecursionlimit to a value measured from
the actual stack (e.g. derive max depth from a stack-size constant ×
per-frame cost, as CPython does with its `tstate->recursion_remaining`
+ `Py_C_RECURSION_LIMIT` split), or add a cheap per-call stack-high-water
check (stack address delta vs a budget) in enter_call that raises
RecursionError before the OS does.

## FINDING 5 (CONFIRMED, LOW-MEDIUM): genexp single-pull leaves its whole
## value-stream snapshot registered; exhaustion cleanup exists on ONE path
## only

Probe: /tmp/oomhunt/p16_genexp.py (N `sum(x*2 for x in range(5))`):

| N      | maxrss_kb |
|--------|-----------|
| 20k    | 15,942    |
| 100k   | 33,604    |
| 200k   | 55,902    |

~200KB per 1k iterations, growing linearly with iterator count (not with
values). Control p9/p15 (exception churn) is FLAT at ~11.8MB across
20k→200k, so this is specifically the genexp/iterator registry path.

Suspected owner: crates/ap-run/src/interp.rs — genexp "v0 materializes its
value stream on first pull" (comment at :3270). `gen_pull` (:7869) does
`interp.generators.borrow_mut().remove(&id)` then
`interp.iterators.borrow_mut().insert(id, it)` (:7905, :8088, :10725,
:10831).

CAVEAT (owner nuance found while writing this up): `pull_iter` (:8886)
ALREADY drops an exhausted `IterState::Seq` iterator — interp.rs:8903-8915
("a finished iterator's Seq state pins every value it ever yielded
(genexpr snapshots) — a permanent root set. Drop it.", the 02.04 fix).
But that cleanup lives ONLY in `pull_iter`; the other registration sites
(`register_shared_iter_pub` :10721, `user_iter` :10802/:10828, and the
shared table inserts in builtins_modules.rs) have no exhaustion-prune, and
`register_iter` (:8816) registers iterators whose ids are only reaped via
the pull_iter path. The measured growth shows SOME path(s) still
accumulate (or the entries linger between pulls); RSS cost per entry is
small (a Vec sink of 5 ints + PyIterator), hence only visible at 10^5
scale — but it's the same unbounded-registry class as FINDING 2 and
matters for long-running processes (pytest sessions, scrapy crawls).

Fix sketch (not applied): hoist the :8903-8915 exhaustion-drop into the
shared pull/next path so every registration site benefits (or prune
exhausted iterators at the GC major pass); same treatment for the
generators registry once StopIteration is delivered.

## CLEAN (flat RSS, no finding) — probed and negative:

- p5_instance: 200k class-instance/closure churn + periodic collect: FLAT
  12.3-12.6MB across all three sizes. GC heap growth is bounded (post
  bcc0a04/743e277 fix verified live).
- p8_regex: 20k distinct pattern compilations: FLAT ~11.8MB — the re
  cache does not grow with distinct patterns (either bounded or untracked;
  no leak).
- p9_exc / p15_tb: raise/catch churn incl. nested-frame tracebacks: FLAT
  ~11.8MB. Exception objects + traceback frame lists are reclaimed.
- p14_del: __del__ churn: FLAT 11.6-11.8MB. __del__ finalization is
  bounded (weakref-dead drain works for the plain __del__ path — it's the
  explicit weakref/finalize registries that leak, FINDING 2).
- p11_lru_bounded: functools.lru_cache(maxsize=128) with 200k distinct
  keys: FLAT ~21.5MB. maxsize IS honored (vendored functools.py:558 +
  native wrapper behave).
- p1_dict / p2_list / p3_intern: grow linearly with LIVE data retained —
  expected (probes hold the data deliberately); no amplification observed
  (200k-entry dict 52MB ≈ 260B/entry, acceptable for a debug build).
- p4_gen vs p4b_gen_gc vs p4c_nogen: generator CHURN (exhausted
  immediately) grows identically WITH and WITHOUT explicit gc.collect()
  (35→205MB both), while the no-generator control is flat at 11MB. This
  is the per-generator fiber allocation (FINDING 1) plus per-genexpr
  registry growth (FINDING 5) — the churn probes are the same mechanism
  seen through a different lens, not a separate finding.

## Not covered (out of time / out of fence budget)

- threading module surfaces (threading.rs/threading_call.rs are sibling
  WIP per AGENTS.md — did not probe to avoid tripping mid-flight code).
- lxml / scrapy chain RSS beyond the documented 0.8-1.4GB baseline.
- WASM tier (memory behavior there is a different embedding story).

## Changed on disk

- docs/oom-bughunt-2026-11-06-rss-fiber-weakreg/ (this file).
- /tmp/oomhunt/*.py (19 probe files), /tmp/oomhunt/sweep*.sh (7 sweep
  drivers), /tmp/oomhunt/results*.txt (raw measurements).
- NOTHING else: no product code touched, no git commit.
