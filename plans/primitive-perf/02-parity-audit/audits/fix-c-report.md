# Fix-leaf report — fix-c (structural): Int interpreter-loop pathology + container-mint floor

Leaf: `fix-c` (primitive-perf campaign, Fix-C structural). Worktree:
`/Users/caimlas/.git/worktrees/leaf-ppfix-fixc` (branch `tier1-ppfix-fixc`,
forked @ main `b23dfd1`). CPython ref: `/opt/homebrew/bin/python3.14` (3.14.7).
All numbers below: this host (M2 Ultra), release builds, quiet window
(loadavg 4-5), best-of-N per the probe conventions.

## Commits (explicit pathspecs per increment)

| commit | what |
|---|---|
| `436a964` | fix(iter): SharedList::next O(1) indexed pull + fixc_bisect.py probe |
| `4c8161a` | fix(gc): gen-1 full-retain live-deregistration bug + is_unboxed edge-scan fast path + t0 700→7000 + mint probes |
| `f91a9a2` | perf(obj): dunders double-allocation fix (PyDict::new_unregistered) |

(An Expr::Call primitive-method fast path was attempted and deliberately NOT
landed — measured nil gain; see honest gaps. An attr early-bind arm was
attempted, caught RED by the stdlib gate (enum.py `classdict.update` →
'NoneType' not callable), and REVERTED within the increment — f91a9a2's tree
contains neither.)

## Bisect findings — what the roots ACTUALLY were

`tests/bench/primitive/probes/fixc_bisect.py` isolates one suspect at a time
(bare range loop / bare list iter / indexed loop / while counter / append loop /
listcomp / range listcomp × N=6k/16k/32k/64k). Verdict at base b23dfd1:

| probe @32k | antipythony | CPython |
|---|---|---|
| bare `for i in range(N)` | 6.0 ms (linear) | 0.53 ms |
| while counter | 7.7 ms (linear) | 0.41 ms |
| indexed `lst[i]` loop | 8.6 ms (linear) | 1.0 ms |
| **bare `for i in lst`** | **1791 ms (superlinear)** | 0.74 ms |
| **`[i for i in lst]`** | **1776 ms (superlinear)** | 0.53 ms |
| `[i for i in range(N)]` | 2.5 ms (linear) | 0.44 ms |

**Root 1 (the "~4,500x bare-loop floor"): `IterState::SharedList::next()`
called `l.items()` on EVERY pull.** For an unboxed (02.09c Smi-storage) list,
`ItemsRef` materializes a FULL boxed `Vec<Value>` snapshot of the whole
container per pull → O(n²) for any for/listcomp over an int list. The
suspects the brief named (per-element Gc minting for Int, GC cascade
frequency, registry insert cost) were all ELIMINATED by the bisect: bare
range/while/indexed loops are linear and healthy (~190ns/step vs CPython
12.6ns; no allocations, no GC involvement). The knee (~20k) matches all three
audit datasets (sort listcomp 17.6s@100k, dict 57µs/elem@32k).

**Root 2 (a real GC bug found while instrumenting the mint floor): gen-1
passes ran the FULL registry retain.** `collect_gen`'s step 6 branched
`gen0 → scoped removal, else → full retain`; gen-1 fell into the else, but
its `reachable` set covers only mid ∪ young — every OLD registry entry
(surviving >1 gen-0 cascade) failed `reachable.contains` and was silently
DEREGISTERED while alive. Probe evidence (`ap_gc_eval`): a 20k held-list mint
loop lost exactly the pre-gen1 entries at each gen1 pass (tracked 15400 →
7700 at i=15400, deterministic; `garbage=0` in the pass — removal without
sweep accounting). This breaks the subtract-refs fence invariant for every
long-lived container (a cycle touching a deregistered object can never be
reclaimed) and churned a full-map retain rebuild per gen-1 pass. Fix: the
full retain is reserved for full passes (gen ≥ 2); gen-0 AND gen-1 delete
only scoped garbage. Retention now exact through 160k mints.

**Root 3 (the "~1µs container-mint floor"): decomposed, not one thing.**
Raw measurements (`ap-gc-eval` probe bin): Gc::new(PyList 3-elem) alone =
48ns; with the registration hook = ~985ns. Split: registry insert ≈
160-250ns (routed TLS probe + drain check + 5-map critical section + ladder);
**collect passes ≈ 400-700ns/mint amortized** (228 auto-passes per 160k
mints; gen-1/gen-2 passes scan growing scopes). Additional mint-floor
finding inside PyObject: `dunders: Mutex::new((*PyDict::new()).clone())`
allocated a REGISTERED dict, deep-cloned it (2nd alloc), dropped the
original — ~600ns waste per PyObject mint (bound methods mint per attribute
read).

## Fixes landed (all general; no benchmark special-casing)

1. **`SharedList::next` O(1) pull** (ap-run/iter.rs): `PyList::get(cur)`
   bounded read on both storage arms (boxed slice index; Smi inline re-box).
   Mutation-during-iteration semantics unchanged (reads see current
   contents; cursor semantics identical).
2. **GC gen-1 retain fix** (ap-gc/gcmod.rs): full retain only on full
   passes; scoped O(garbage) removal for gen-0 and gen-1.
3. **`edges_of` unboxed-list fast path** (ap-gc/gcmod.rs +
   `PyList::is_unboxed`): an unboxed list has ZERO internal edges by the
   02.09c fence invariant (raw i64 elements carry no registry addresses) —
   skip the boxed-snapshot materialization per scanned list per pass. Fence
   invariant preserved (documented; gc_differential + ast_getattr_identity_fence
   green).
4. **Default t0 700 → 7000** (ap-gc/gcmod.rs, documented divergence): the
   per-pass cost here is ~1-2µs/object vs CPython's ~10ns; at t0=700 the
   amortized GC tax was 3-8x of allocation-heavy wall-clock. `gc.set_threshold`
   and `APX_GC_THRESHOLD` still respected verbatim; the corpus pins only the
   threshold's shape/round-trip (gc016), never timing (C-23/D1).
5. **`PyDict::new_unregistered` + PyObject dunders** (ap-obj): kills the
   double allocation in every PyObject mint. Fence status unchanged
   (dunders was already unregistered post-clone; parent is the registered
   container; `edges_of` walks dunders under the parent's lock).

## Before/after per target (same probes as the source datasets)

| target | before | after | note |
|---|---|---|---|
| bare int loop 32k | 57,100 ns/elem (1.83s) | **188 ns/elem (6.0ms)** | ≥50x target MET (304x); floor = interpreter walk, linear |
| 100k int listcomp | 17.6 s | **~8 ms** | ≤1s target MET (2200x); ratio vs CPython 8.2ms/0.5ms = 16x |
| json.loads dict-heavy | 0.22x CPython | **0.33x @1MiB / 0.50x @48KiB** | P10 rows; 1.5-2.3x improved, target 1.0x NOT met (below) |
| d[k] getitem loop 32k | 2,027x | **14.7x** (P09 row) / 15.8x (probe) | <100x target MET |
| dict.build 32k | 3,106x | **6.6x** (P09 row) / 11.8x (probe) | |
| dict.get 32k | 2,104x | **111x** (P09 row) / 19.8x (probe) | residual = bound-method dispatch per step (below) |
| dict.iter 32k | 13.8x | 13.0x | unchanged (was already container-scan bound) |
| sorted(list[int]) 100k | ~1,700x | **2.6x slower (0.38x row)** | the sort int-pathology was Root 1; the raw list.sort arm stays 4-8x |

Bench slices (quiet window, this build): `/tmp/fixc_p09.json`,
`/tmp/fixc_p10.json`, `/tmp/fixc_p08.json` (scratch paths).

## Honest gaps / next bottlenecks

1. **json.loads 1.0x NOT met (0.33x @1MiB).** Kernels remain 1.4-2.2x faster
   than CPython (ppfix-json data); the residual is per-container mint
   (~985ns vs CPython's ~150ns) + per-value boxing. The gen-1 fix + t0
   change cut the GC share of that floor; what remains is Arc-alloc +
   registry-insert structure. Next lever: registry batching (thread-local
   pending set flushed at the ladder) or a bump-allocated nursery — both
   ap-gc structural work beyond this leaf's risk budget.
2. **Bound-method dispatch (`dict.get` row, 111x; `l.append` loop 1.79µs/
   read vs bind-once 0.32µs).** Per-attribute-read BoundMethod MINT is the
   cost (PyObject = Gc alloc + registered attrs dict + ~11 locks). Attempts:
   call fast path (nil — cost isn't call_value), attr early-bind arm
   (UNSOUND: broke vendored enum.py via intermediate-arm semantics, reverted,
   caught by the stdlib gate at 31/48), dunders fix (26% win, landed).
   Next lever: lazy attrs (`Mutex<Option<PyDict>>` in PyObject) — large
   surface (~50+ access sites), filed.
3. **Interpreter walk floor ~190ns/step vs CPython 12.6ns** (bare range
   loop 5.8ms@32k). This is the tier-0 tree-walker itself (Expr eval, frame
   ops per step) — structural tier-1 territory (02.05), not an allocation
   problem. Out of scope for this leaf.
4. **Sibling workspace leftovers left untouched**: uncommitted
   `Cargo.lock`, `builtins_modules.rs` (fmt + call_random rework),
   `FUZZ_SEEDS.txt` diffs present in the worktree — NOT this leaf's work,
   not committed (fix-sort honest-gap #4 discipline).

## Gates (all re-run on the final tree f91a9a2)

| gate | result |
|---|---|
| fix_int.py | GREEN (rc 0) |
| fix_json.py (127 cases) | GREEN (0 mismatches) |
| fix_sort.py (42 cases) | GREEN (0 mismatches) |
| fix_sha_zlib.py (positional) | GREEN (rc 0) |
| fix_random.py | GREEN |
| diff_b64.py (2132 cases) | GREEN (0 mismatches) |
| diff_hex.py (1976 cases) | GREEN (0 mismatches) |
| ap-conform values | 50/50 |
| ap-conform stdlib | 48/48 |
| cargo test -p ap-run --release | 208 passed / 0 failed (incl. gc_differential + ast_getattr_identity_fence) |
| cargo test -p ap-obj -p ap-gc --release | 107 passed / 0 failed |

## Repro

```
cd /Users/caimlas/.git/worktrees/leaf-ppfix-fixc
export CARGO_BUILD_JOBS=4 CARGO_TARGET_DIR=$PWD/target-fixc
cargo build --release -p ap-cli -p ap-conform
./target-fixc/release/antipythony tests/bench/primitive/probes/fixc_bisect.py all
./target-fixc/release/antipythony tests/bench/primitive/probes/fixc_mint.py
./target-fixc/release/antipythony tests/bench/primitive/probes/fixc_dict.py
./target-fixc/release/ap_gc_eval                      # raw mint/GC floor probe
/opt/homebrew/bin/python3.14 tests/bench/primitive/fix_json.py --other $PWD/target-fixc/release/antipythony run
```
