# HOT-PATH-PARITY — master plan

**Goal:** No Python primitive may be slower than CPython 3.14 on any
workload. The primitive-perf campaign closed the algorithmic gaps; what
remains is allocation and dispatch overhead concentrated on the
HOT-PATH primitives (dict ops, str join/split, json.loads). This plan
closes those with allocation-shape and dispatch-shape changes that are
invisible to Python semantics — byte-exactness stays the gate.

**Machine:** Mac M2 Ultra (24-core, 64 GB), aarch64. **Reference:**
python3.14 (/opt/homebrew/bin/python3.14, 3.14.7). **Gates:** every
leaf re-runs the 7 diff gates + ap-conform values 50/50 + stdlib
48/48 + cargo test -p ap-run --release. One heavy cargo lane;
CARGO_BUILD_JOBS=4; worktrees under ~/.git/worktrees/, CARGO_TARGET_DIR
per leaf.

## Where the remaining slowness lives (measured, from fix-c-report.md)

| root cost | measured | gates which rows |
|---|---|---|
| GC registry insert ~985ns/container (vs CPython ~150ns) | json.loads 0.31-0.46x, dict.build, every output mint | json.loads, dict rows, allocation-heavy loops |
| BoundMethod mint ~1.5µs/call (PyObject = Gc alloc + registered attrs dict + ~11 locks) | dict.get -99%, all method calls | dict.get, method-heavy loops |
| tier-0 walk floor ~190ns/step (vs CPython ~13ns) | sort probes, listcomp shapes, every loop | list.sort/sorted, join/split harnesses |
| misc per-primitive tails | to_bytes BE double-pass, str.split per-piece alloc | int.to_bytes, str.split |

## Tree

```
plans/hot-path-parity/
  master.md                              # this file
  01-nursery/
    orchestrator.md
    01-gc-nursery.md                     # bump arena + batch registration (GC)
    02-lazy-dunders.md                   # PyObject dunders -> lazy (ap-obj)
  02-dispatch/
    orchestrator.md
    01-inline-caches.md                  # per-site method/attr resolution cache
    02-typed-dict-regions.md             # typed dict getitem/setitem in tier-1 (large)
  03-jit-for-loops/
    orchestrator.md
    01-for-loop-lowering.md              # Stmt::For in cfg::lower_fn (the gate)
    02-osr-liveness.md                   # backward dataflow liveness (small)
  04-micro/
    orchestrator.md
    01-to-bytes-be.md                    # BE single-pass emit + str.split fast path
```

## Dispatch protocol

1. `01-nursery` first (both leaves parallel — file-disjoint: ap-gc vs
   ap-obj). Biggest single lever: ~985ns -> ~50ns per container mint.
2. `02-dispatch/01-inline-caches` after 01-nursery lands (cache design
   reads the post-nursery mint shape). `02-dispatch/02-typed-dict-
   regions` is LARGE (4-8 sessions) — dispatch only after 03 lands and
   re-measures; expected win is bounded by remaining walk floor.
3. `03-jit-for-loops` parallel with 01-nursery (file-disjoint:
   ap-codegen-native vs ap-gc/ap-obj). 01-for-loop-lowering is THE
   gate for sort/listcomp/join shapes; 02-osr-liveness bundles with it.
4. `04-micro` opportunistic filler (small leaf, any idle lane).
5. Every leaf: bisect-first, general fixes only, no benchmark
   special-casing; GC leaves must run the GC differential suites
   (gc_differential, ast_getattr_identity_fence) — the fence invariant
   (unboxed => zero internal edges + stays registered) is load-bearing.

## Interface contracts

- Allocation API: nursery expose `ap_gc::alloc_room(len) -> &mut [u8]`
  style bulk-room API; containers get `new_in(nursery)` variants;
  promotion on first pointer store OUT of the arena range.
- Inline caches: per-site one-entry cache keyed by receiver type id;
  miss falls to the existing full lookup (no semantic change); cache
  invalidation on class mutation via the existing type_version counters
  (02.05b infrastructure).
- Every diff gate, conform slice, and cargo suite green BEFORE perf
  claims; bench slices to scratch paths; orchestrator merges rows.

## Completion Tracking Table

| group | status |
|---|---|
| 01-nursery | PENDING |
| 02-dispatch | PENDING (01 after nursery; 02 gated on re-measure) |
| 03-jit-for-loops | PENDING |
| 04-micro | PENDING |

## Exit gate

Closing bench on merged trunk: NO primitive row below 1.0x CPython on
either workload size, or a documented deferral naming the structural
floor (tier-0 walk) with the tier-1 ticket that removes it. Report
refresh + README figures update.

## Review Checklist

- [ ] in-session review after every child return
- [ ] GC fence invariants re-proven on nursery leaves
- [ ] single-writer rule on bench_report.json (orchestrator merges)

## Test Gate

Every leaf carries: 7 diff gates green + conform slices + cargo suites
+ GC differentials (nursery leaves) before perf claims.

## Coding Conventions

- No new crate deps (master dependency posture; user approvals do not
  extend here).
- Rust: unsafe only with a proven-invariant comment naming the proof;
  aarch64 guards + scalar fallback for any SIMD.
- Machine policy: ONE heavy cargo lane; CARGO_BUILD_JOBS=4.
