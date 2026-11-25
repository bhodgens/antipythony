# Fix leaf ppfix-sort — 3 sort correctness bugs, CPython-exact error semantics

Worktree: `/Users/caimlas/.git/worktrees/leaf-ppfix-sort` (branch
`tier1-ppfix-sort`, base 4141a86). CPython ref: /opt/homebrew/bin/python3.14
(3.14.7). Parent audit: `sort-dict-json.md` (this directory).

## What was fixed

### Bug 1 — sorted() swallowed mixed-type compare TypeErrors (both arms)

- Symptom (verified RED before fix): `sorted([3,'a',1])` → `[3,'a',1]`
  input order; `sorted([1,2,'x',None])` → unchanged. CPython raises
  `TypeError: '<' not supported between instances of 'str' and 'int'`.
- Root cause: every comparator closure in `builtin_sorted`
  (crates/ap-run/src/builtins.rs) ended `Err(_) => Ordering::Equal`, and
  the kwarg-capable list.sort arm (builtins_modules.rs pylist "sort")
  used `py_lt(..).unwrap_or(false)` + `py_eq` fallback — both turned the
  compare error into a silent Equal.
- Fix: all sorted() arms and the pylist generic arm now PROPAGATE the
  `Flow` (parked `Option<Flow>` cell checked at the top of each
  comparator, re-raised after `sort_by` returns — sort_by can't
  short-circuit). `py_lt` re-wraps user-instance operands with
  `display_type_name` so the class name prints (values_cmp prints the
  static `'object'`; c027's class-name text now pinned on the sort path
  too — probe: `sorted([Q(), 2])` → `'<' not supported between instances
  of 'int' and 'Q'`, byte-identical with CPython).

### Bug 2 — list.sort() v0 arm silently no-oped mixed lists

- Symptom (verified RED): `[3,'a',1].sort()` returned None, list
  unchanged, NO error.
- Root cause: methods.rs v0 arm pre-scanned all-int/all-str and sorted
  raw; anything else was left untouched with no error. The arm carries
  no kwargs and returns `Some(Ok(Value::None))` unconditionally, so the
  kwargs-capable generic arm never saw the call.
- Fix: the v0 raw fast path is UNCHANGED for all-int/all-str (perf: this
  is CPython's unsafe_long/unsafe_string_compare strategy, 5.2x of
  CPython — do not slow it). For mixed lists the arm now returns `None`
  (= not routed), which falls through the BoundMethod dotted route to
  the pylist "sort" generic arm in builtins_modules — real kwargs,
  strict comparators, CPython-exact TypeError. Non-homogeneous
  orderable mixes (`[1, 2.5, 3]`, `[True, 0, 2.5]`) now sort correctly
  through the same path (verified byte-identical: bool sorts before int
  sorts before float).

### Bug 3 — key=None crashed ('NoneType' object is not callable)

- Symptom (verified RED): `sorted(x, key=None)` and `l.sort(key=None)`
  both raised `'NoneType' object is not callable` — AND corrupted the
  running frame (subsequent calls/print crashed with the same error; the
  audit's `sort(reverse=False)` frame-corruption oddity was this same
  seam: builtins_modules `call_named(None, ..)` raised a Flow mid-arm).
- Root cause: builtin_sorted treated `Some(None)` as a key fn; the
  pylist arm called `callable_name(None)` which raises.
- Fix: both arms treat `key=None` as NO key (CPython listobject.c:
  None never reaches the must-be-callable check). Frame corruption gone
  (probe: full script continues after key=None sort).

### Bonus fixes (CPython-exactness found while in the files)

- Unknown sort kwargs now raise CPython's exact
  `sort() got an unexpected keyword argument 'bogus'` on BOTH sorted()
  and list.sort() (sorted's divergent
  `'<k>' is an invalid keyword argument for sorted()` retired; old
  list.sort bad-kwarg call SILENTLY no-oped — now raises).
- reverse=True tie-stability: CPython reverses the KEYS array
  pre-merge, so equal keys keep INPUT order under reverse=True. The old
  flip-comparator scheme (sorted) and the keyed.reverse() +
  reverse-iteration cancellation (pylist arm) both put equal-key groups
  in reversed input order in some shapes. Replaced with
  decorate-reverse-sort-reverse on both arms; differential-verified
  (`E.stable.rev` / `E.stable.lsort_rev` cases).

## Files touched

| file | change |
|------|--------|
| `tests/bench/primitive/fix_sort.py` | NEW differential gate (42 cases; order AND exception type+text byte-identical vs CPython 3.14; seeded LCG patterns — NO random.Random) |
| `crates/ap-run/src/builtins.rs` | builtin_sorted: key=None, Err propagation ×4 arms, decorate-reverse-sort-reverse, CPython kwarg text; dead f_format_invalid_kw removed |
| `crates/ap-run/src/builtins_modules.rs` | pylist sort generic arm: key=None, strict py_lt with Err propagation, reverse scheme, kwarg text; py_lt re-wraps user-instance TypeError text |
| `crates/ap-run/src/methods.rs` | v0 list.sort: mixed lists fall through to the generic arm (None return) instead of silent no-op; raw fast path untouched |

## Commits

- `ae4155e` test(primitive-perf): fix_sort differential probe (RED) —
  gate lands FIRST, 12/42 cases mismatching (the 3 bugs + 2 kwarg-text
  + 1 userlt-error rows) before any impl change.
- `3238340` fix(sort): ppfix-sort — 3 sort correctness bugs,
  CPython-exact error semantics.

## Gates (all re-verified on the final tree)

| gate | result |
|------|--------|
| (a) fix_sort.py vs CPython 3.14 | GREEN — 42/42, 0 mismatches (RED proven at ae4155e) |
| (b) ap-conform values | 50/50 passed |
| (b) ap-conform stdlib | 48/48 passed |
| (c) cargo test -p ap-run --release | 18 suites, 0 failed (debug→release symlink gotcha hit as documented: ap-run-probe + ap-run-eval symlinked from target-ppfix-sort/debug/ → ../release/) |
| (d) timing spot-check | see below |

## (d) Timing spot-check (this host, M2 Ultra, release build, best-of-3;
NOT the shared bench_report.json)

sorted() over 100k seeded ints + 100k seeded strs (seeded LCG, no
random.Random), ours vs CPython 3.14:

| probe | antipythony | CPython | ratio |
|-------|-------------|---------|-------|
| sorted() int @100k (generic path) | 17.7-18.2 s | 10.4-13.1 ms | ~1,700x slower |
| sorted() str @100k (generic path) | 63-77 ms | 17.5-18.1 ms | ~4x slower |
| sorted() float @100k (generic path) | 43.7 ms | 11.2 ms | ~3.9x slower |
| list.sort() int @100k (raw arm) | 39.8-50.9 ms | 10.6-11.0 ms | ~4.4x slower |
| list.sort() str @100k (raw arm) | 59-75 ms | 17.7-18.0 ms | ~3.8x slower |

### Int-pathology bisect (Fix-C handoff data — root NOT fixed here, scoped out)

The 1,700x int row is NOT the sort dispatch: it is the audit's known
Int-loop pathology, and the bisect NARROWS it further than the audit
could:

- 10-element int sorted ×1000: 2.6 ms (≈2.6µs/call) vs float 2.3 ms —
  fine. The pathology is SIZE-dependent, not per-compare.
- 10k: sort 190 ms; 50k: sort 4.47 s; 100k: 17.5 s — SUPERLINEAR
  (10x size → ~90x time; n log n would predict 10x). Suspect: O(n²)
  behavior in the sorted() int path. NOT in the comparator: the same
  list via `.sort()` (raw arm, no Python dispatch) is 19 ms @50k.
- `[i for i in ints]` listcomp over 100k ints: 17.6 s (the same
  superlinearity appears in a PURE LISTCOMP with no sort at all —
  matches the audit's "listcomp_int_20k = 0.709s" row exactly).
- Build-loop (append per iteration) over 100k ints: 265 ms — fine.
- So the pathology is in listcomp/repeated-append-to-large-list on
  `Value::Int` (unboxed-Smi demotion → boxed-bigint Gc churn?,
  growth-triggered GC interaction?), NOT in sort. list.sort(ints)
  (the raw arm) is 4.4x CPython — healthy. Handing the bisect data to
  the Fix-C owner: reproduce with `[i for i in range(100000)]`-shaped
  int listcomps, profile `PyList::append` on Int beyond ~20k elements.

### Perf (secondary goal): NOT attempted beyond the correctness-driven restructure

The special_lookup-per-compare hoist (audit Fix B) was NOT done — the
time budget went to the error-semantics surface (which was larger than
estimated: the frame-corruption seam, reverse tie-stability, and kwarg
texts all needed CPython-exact treatment to keep the differential gate
byte-identical). The generic path remains per-compare `py_lt` (4x MRO
walk). Honest gap, scoped out; the str/float generic rows (~4x) are the
proxy for what Fix B can recover.

## Honest gaps

1. Int-listcomp/sorted-int superlinear pathology (above) — root
   identified as NOT-in-sort, bisect data recorded; belongs to Fix C /
   interpreter-loop leaf.
2. Per-compare special_lookup hoist (Fix B) not attempted.
3. `co_argcount`/`co_nlocals`/frame-attr divergences found during
   probing (frame_probe54-68 in /tmp) — pre-existing, UNRELATED to
   sort, not fixed, not gated here.
4. FUZZ_SEEDS.txt had uncommitted sibling additions in the worktree —
   left untouched (not mine, not committed by this leaf).

## Repro commands

```
cd /Users/caimlas/.git/worktrees/leaf-ppfix-sort
export CARGO_BUILD_JOBS=4 CARGO_TARGET_DIR=$PWD/target-ppfix-sort
cargo build --release -p ap-cli -p ap-conform
/opt/homebrew/bin/python3.14 tests/bench/primitive/fix_sort.py \
  --other $PWD/target-ppfix-sort/release/antipythony
./target-ppfix-sort/release/ap-conform run \
  --corpus 'tests/corpus/values/*.py' --impl './target-ppfix-sort/release/antipythony'
./target-ppfix-sort/release/ap-conform run \
  --corpus 'tests/corpus/stdlib/*.py' --impl './target-ppfix-sort/release/antipythony'
cargo test -p ap-run --release   # needs debug/ap-run-probe + debug/ap-run-eval symlinks → ../release/
```
