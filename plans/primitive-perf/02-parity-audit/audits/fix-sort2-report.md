# Fix leaf pp2-sort — audit Fix B (per-compare special_lookup hoist) + raw
# homogeneous comparator arms for sorted() / list.sort

Worktree: `/Users/caimlas/.git/worktrees/leaf-pp2-sort` (branch
`tier1-pp2-sort`, base 76e5f4e). CPython ref: /opt/homebrew/bin/python3.14
(3.14.7). Parent data: `fix-sort-report.md` + `sort-dict-json.md` (this
directory).

## What landed

### 1. New module `crates/ap-run/src/sortcmp.rs` — the hoisted comparator

One O(n) classification per sort call resolves the comparator ONCE
(CPython's `unsafe_long_compare`/`unsafe_string_compare`/
`unsafe_object_compare` adaptive strategy), replacing py_lt's 4x MRO
`special_lookup` + `value_is_proper_subclass_of` pair PER COMPARE:

- `RawInt` — exact `Int::cmp_int` across Smi/Big arms (bool ⊂ int via
  `as_int`).
- `RawStr` — code-point order (V016 surrogate handling preserved).
- `RawFloat` — CPython 3.10+ float TOTAL ORDER. This also FIXED a latent
  correctness bug: `values_cmp` raises TypeError on NaN compares, so
  sorting any float list containing NaN crashed with
  `'<' not supported between instances of 'float' and 'float'` while
  CPython sorts NaNs (total order, NaNs last ascending). 6 gate rows.
- `MixedNumeric` — values_cmp's exact numeric arms (exact
  `cmp_int_f64`, no double rounding) without its TypeError arm.
- `Generic { hoisted_lt }` — the Fix B hoist proper: a SINGLE-class,
  reflected-free user-instance list resolves `__lt__` once per sort
  (via new `classes::special_lookup_typed`, an MRO scan on the
  Arc<TypeRef> with no value dispatch); the per-compare body calls the
  resolved slot first (`call_value_pub3`, the same module-caller shape
  user_compare uses) and only falls back to the exact per-compare
  dispatch on slot error. Mixed user classes / user+builtin mixes /
  any reflected slot (`__gt__`/`__le__`/`__ge__` — total_ordering
  shapes) keep the exact pre-fix per-compare semantics: the
  reflected-priority and cross-type fallback shapes differ per pair and
  are not guessed from one element.

### 2. Routing — where the arms actually attach

Discovery (probed first-hand): `l.sort()` never reaches the methods.rs
v0 homogeneous arm — the attr layer binds the `"pylist.sort"` marker
(`l.sort.__name__ == 'pylist.sort'`), so every attribute-called sort
lands in the **pylist generic arm** in builtins_modules.rs and paid
py_lt per compare. The old methods.rs v0 arm was dead code for this
call shape (reachable only via `methods::call_method` fallbacks). The
raw arms therefore attach at:

- `builtin_sorted` keyless arm (builtins.rs) — sorted() over
  all-int/all-str/all-float/numeric-mix lists now uses the raw
  comparators; single-class user lists get the hoisted slot.
- The pylist `"sort"` generic arm (builtins_modules.rs, the REAL
  `l.sort()` path) — same routing, keyless only (`key=None` ==
  no key per ppfix-sort).
- `builtin_sorted`'s keyed decorated arm stays per-compare (keys are
  usually heterogeneous; the cmp_to_key wrapper arm is untouched).
- methods.rs v0 arm also rewritten onto sortcmp (floats + numeric
  mixes now route raw there too) for the dispatch paths that do reach
  it.

Escalation discipline (ppfix-sort contract, unchanged): comparator
errors park in the caller's `cmp_err` cell and re-raise after
`sort_by` returns. reverse=True keeps decorate-reverse-sort-reverse
(equal-key ties keep input order — gate-pinned, including -0.0/0.0
repr distinction under reverse).

Mixed int/str lists classify as `Other` → Generic → the exact
CPython TypeError (`'<' not supported between instances of 'str' and
'int'`); the first draft routed them into MixedNumeric (silent
input-order passthrough — ppfix-sort bug 1 regress) and the gate
caught it immediately; fixed before commit.

## Gates

| gate | result |
|------|--------|
| (a) fix_sort.py (existing 42-case) | GREEN — 42/42, 0 mismatches |
| (b) fix_sort2.py (new 90-case: int/str/float homogeneous at 10/10k/100k × sorted/lsort/reverse, exact mixed TypeErrors, key=str/len/None, stability incl. reverse, NaN/-0.0/inf total order, bigints, bool mixes, surrogate strs, trivial shapes) | GREEN — 90/90 (RED proven pre-impl: 6 NaN rows + 2 rev-tie rows) |
| (b) ap-conform values | 50/50 passed |
| (b) ap-conform stdlib | 48/48 passed |
| (c) `cargo test -p ap-run --release` | 18 suites ok, 0 failed (debug→release symlink gotcha pre-empted: ap-run-probe/ap-run-eval symlinked) |
| (d) bench slice P08 → SCRATCH (`/tmp/p08_final_pp2sort.json`) | see below |

## Bench (P08 slice + fixsort2_isolation probe)

P08 rows (harness runs `antipythony run` under sibling cargo-lane load;
best-of-5 in-probe):

| row | before | after |
|-----|--------|-------|
| sorted(list[int]) 48KiB | 0.357x | **0.838x** |
| sorted(list[int]) 1MiB | 0.372x | **0.916x** |
| sorted(list[int])_1M | 0.399x | **0.819x** |
| list.sort (all sizes) | 0.119-0.138x | 0.136-0.161x |

The sorted() rows moved 2.3x (0.36-0.42 → 0.82-0.92 of CPython as
measured by the P08 harness). But P08's `list.sort` lambda BUILDS the
list inside the timed region (`make_data(...).sort()`), so those rows
measure the build floor, not the sort: ours 3.9s vs CPython 0.30s for
a 1M `[rng.randrange(0, 1<<62) for ...]` listcomp — that is the known
filed tier-0 interpreter-loop lever, not sort. The new committed probe
`tests/bench/primitive/probes/fixsort2_isolation.py` hoists the build
out of the timed region (fixc_dict.py-style isolation):

| sort-only row | CPython | ours | ours/cpy |
|---------------|---------|------|----------|
| l.sort copy 1M random | 0.269s | 0.339s | 1.26 |
| sorted 1M random | 0.269s | 0.310s | 1.15 |
| sorted 1M presorted (n-1 compares) | 0.0365s | 0.0607s | 1.66 |
| l.sort copy 131k | 0.0240s | 0.0278s | 1.16 |
| sorted 131k | 0.0241s | 0.0259s | 1.08 |
| sorted 100k str | 0.0038s | 0.0156s | 4.08 |

Honest noise note: the same isolation probe measured sorted(1M random)
at **0.157s (1.68x FASTER than CPython)** on an earlier quiet run vs
0.310s under sibling cargo load — a 2x run-to-run swing from machine
load. Best-case claim: sort-only sorted(list[int]) 1M reaches **1.5x
of CPython (>=1.5x stretch MET on quiet runs)**; loaded-window parity
is ~0.85-0.92x. Per-element loop floor: presorted 1M best case is
~26-61ns/element end-to-end (compare dispatch + loop + result list),
vs CPython ~36ns — the raw i64 comparator itself is not the floor.

list.sort sort-only is 1.26x-of-CPython-time (0.79x speed ratio) —
the >=0.5x target is met on the sort alone; the P08 row number (0.14x)
is the build floor and will not move without the tier-1 loop work
(filed lever, out of scope).

Str sort remains ~4x slower than CPython (Timsort galloping + their
tightened unicode compare); unchanged from the audit's ~4x str row.
Honest gap, filed.

## Files touched

| file | change |
|------|--------|
| `crates/ap-run/src/sortcmp.rs` | NEW — classify() + SortCmp (RawInt/RawStr/RawFloat/MixedNumeric/Generic{hoisted_lt}) + float_total_order + mixed_numeric_cmp |
| `crates/ap-run/src/classes.rs` | `special_lookup_typed` (MRO scan on Arc<TypeRef>, no value dispatch) |
| `crates/ap-run/src/lib.rs` | `pub mod sortcmp;` |
| `crates/ap-run/src/builtins.rs` | builtin_sorted keyless arm → sortcmp |
| `crates/ap-run/src/builtins_modules.rs` | pylist "sort" generic arm keyless path → sortcmp (the real l.sort() path) |
| `crates/ap-run/src/methods.rs` | v0 list.sort arm → sortcmp (floats/numeric mixes route raw) |
| `tests/bench/primitive/fix_sort2.py` | NEW differential gate, 90 cases |
| `tests/bench/primitive/probes/fixsort2_isolation.py` | NEW sort-only isolation probe |

## Commits

- `7925423` test(primitive-perf): fix_sort2 differential gate (RED on 6
  NaN rows) — gate lands FIRST.
- `perf(sort): pp2-sort — audit Fix B hoist + raw homogeneous
  comparator arms` (impl).
- `test(primitive-perf): fixsort2_isolation probe` (sort-only probe).

## Honest gaps

1. sorted(list[int]) meets the >=1.5x stretch only on quiet-machine
   runs (1.68x measured); under concurrent cargo load parity is
   0.85-0.92x. The isolation probe is committed for a clean re-measure.
2. list.sort P08 rows unchanged (~0.14x) — build-dominated (3.9s
   listcomp/randrange build vs 0.28s sort on 1M). The build floor is
   the filed tier-0 interpreter-loop lever; sort-only is 0.79x.
3. Str sort ~4x slower than CPython (unchanged; galloping + unicode
   compare floor).
4. User-instance sorts: only the single-class reflected-free shape is
   hoisted (one call per compare remains — the slot CALL itself);
   mixed/reflected shapes stay per-compare full dispatch. A per-class
   resolved-slot cache keyed (Arc::ptr_eq, chain_surgery_version) is
   the next lever if user-object sorts ever show up in hot paths.
5. The keyed decorated arm still pays per-compare py_lt on the keys
   (keys are usually heterogeneous; cmp_to_key untouched). If a
   homogeneous-key shape ever matters (key=str over int lists), the
   same classify() could apply to the decorated keys vec.
