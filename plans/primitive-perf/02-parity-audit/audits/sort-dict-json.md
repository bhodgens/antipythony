# Audit — P08 sort + P09 dict + P10 json (leaf 02-parity-audit/03)

Audit-only leaf. No implementation code touched, nothing committed.
Worktree: `/Users/caimlas/.git/worktrees/leaf-primperf` (tier1-primperf).
CPython ref: `/opt/homebrew/bin/python3.14` (3.14.7). All file:line cites
read first-hand in this worktree. Baseline `bench_report.json` arrived
mid-audit (2026-11-24T20:47Z run); P08/P09 rows are ERROR, P10 has NO
rows — hand measurements below fill the gap, orchestrator reconciles.

Measured ratio summary (this host, M2 Ultra, release binaries; hand-run
best-of-N probes in /tmp, NOT the harness runner):

| row | probe | antipythony | CPython | ratio |
|-----|-------|-------------|---------|-------|
| P08 | sorted() int @20k | 0.716 s | 0.170 ms | ~4,200x slower |
| P08 | sorted() int @100k | 17.2 s | 0.848 ms | ~20,000x slower |
| P08 | list.sort() int @100k | 3.79 ms | 0.726 ms | 5.2x slower |
| P08 | sorted() str @50k | 49.3 ms | 2.73 ms | 18x slower |
| P08 | sorted() float @20k | 3.71 ms | 0.23 ms | 16x slower |
| P09 | dict build (int dictcomp) @20k | 0.713 s | 0.373 ms | ~1,900x slower |
| P09 | dict build (str dictcomp) @20k | 3.06 ms | 0.59 ms | 5.2x slower |
| P09 | d.get loop (str) @20k | 83.8 ms | 0.62 ms | 135x slower |
| P10 | json.dumps @70KB | 0.359 ms | 0.326 ms | 0.91x (≈parity) |
| P10 | json.dumps @280KB | 1.263 ms | 1.288 ms | 1.02x (PASS) |
| P10 | json.loads @70KB | 1.489 ms | 0.331 ms | 0.22x (4.5x slower) |
| P10 | json.loads @280KB | 6.106 ms | 1.417 ms | 0.23x (4.3x slower) |

Harness `bench_report.json` cross-check (read this session):
- P08: 4 rows, all `ERROR` — probe crashes: `random.Random` missing on
  our native `random` module (probe authoring bug + a real random-surface
  gap; note our probe otherwise only needs `randrange`).
- P09: 4 rows, all `ERROR` — `TIMEOUT after 120s`. Consistent with the
  interpreter-loop pathology below (1M-element module-level listcomp +
  1M dispatched `d.get` calls at ~1.5-35µs/element dwarfs 120s? — the
  1M listcomp alone measures ≈35s extrapolated; the lookups loop with
  per-call bound-method dispatch explains the rest). Slow-not-hung.
- P10: **zero rows in bench_report.json** — but
  `00-harness/baseline-notes.md:47-48` claims "P10 | json.dumps/loads |
  beat CPython (PASS both sizes)". That PASS claim is UNGROUNDED in the
  report as it stands (the 50-row checklist in the notes counts rows
  that do not include P10). Orchestrator: either re-run P10 through the
  runner or adopt the hand measurements above. Hand result: dumps ≈
  parity/PASS; loads 4.3-4.5x slower than CPython = FAIL.

---

## 1. sort (P08)

### Actual state

Two distinct arms:

1. **`sorted()`** — `crates/ap-run/src/builtins.rs:2867`
   `builtin_sorted`. Collects via `iter::collect`, then Rust
   `Vec::sort_by` (std stable sort = TimSort-family/driftsort — the
   ALGORITHM is already SOTA-class) with a comparator closure calling
   `py_lt_public` per compare
   (builtins.rs:2902-2948). `key=` decorates via
   `interp::call_value_pub` per element (builtins.rs:2880-2887) — same
   shape as CPython's decorate-sort-undecorate.

2. **`list.sort()`** — TWO arms:
   - `crates/ap-run/src/methods.rs:1937` (`(Value::List(l), "sort")` in
     `methods::call_method`, reached first from the BoundMethod arm at
     `interp.rs:8561`): "v0 homogeneous" fast path — pre-scans
     all-int/all-str, sorts with RAW `i64`/`str` comparisons
     (methods.rs:1947-1953), zero Python dispatch. This is exactly
     CPython's `unsafe_long_compare`/`unsafe_string_compare` strategy.
   - `crates/ap-run/src/builtins_modules.rs:17554` (`("pylist",
     "sort")` in `call_builtin_values`, the kwargs-capable arm):
     cmp_to_key wrappers (17560), key= via `call_named` per element,
     then `sort_by` with `py_lt`/`py_eq` (17636-17647).

### Dispatch cost vs algorithm cost (required separation)

- **Algorithm cost is NOT the problem.** `list.sort()` on 100k random
  i64-range ints: 3.79 ms vs CPython 0.726 ms → the raw-comparator
  stable sort is within 5.2x of CPython's timsort (which benefits from
  40+ years of tuning + galloping; 5x is close for a first fast path).
  No algorithm swap needed. Rust `sort_unstable` (pdqsort) is available
  if stability can be decoupled, but CPython sort is STABLE by contract —
  `sort_unstable` is **not** a drop-in; stay on the stable sort.
- **Dispatch cost dominates the generic path.** Every `py_lt_public`
  compare does: `user_lt_public` → `user_compare`
  (interp.rs:17592) → **four** `special_lookup` MRO walks (a_slot,
  b_slot, both reflected slots, classes.rs:3027) before falling through
  to `values_cmp`. Measured per-compare overhead: str sorted() @50k =
  18x CPython (≈60ns/compare of dispatch on top of a ~10ns compare);
  int sorted() @20k/100k = 4,200-20,000x CPython.
- **Int-pathology flag (honest: root NOT identified, two attempts
  spent).** sorted() over int lists costs ≈0.71 s @20k REGARDLESS of
  presortedness (asc/desc/nearly/random all 0.71 s) — and identical to
  the cost of a 20k int listcomp (0.709 s) and a 20k int dictcomp
  (0.713 s), while float equivalents run 200x faster (3.7 ms). So
  per-element interpreter work on `Value::Int` is ~35µs/element vs
  ~0.15µs for floats. The compare dispatch alone (~60ns × 287k
  compares ≈ 17ms) cannot explain 0.71 s. Suspects for the follow-up
  perf leaf to profile: `Int` representation churn in
  `as_int()`/`cmp_int` on non-Smi bigints, per-op interpreter
  bookkeeping on Int arms, GC churn from boxed Big limbs. This
  pathology ALSO explains the P09 int rows (below) and likely much of
  the P08 ERROR row crisis. It is an INTERPRETER bug/inefficiency, not
  a sort algorithm problem.

### Correctness bugs found while auditing (blockers for any perf work)

1. **TypeError swallowed** — every sort comparator ends
   `Err(_) => Ordering::Equal` (builtins.rs:2910/2921/2937/2946), so a
   mixed-type sort returns the input order SILENTLY:
   `sorted([3,"a",1])` → `[3,'a',1]`; CPython raises
   `TypeError: '<' not supported...`. Probed live on this worktree's
   release binary. (also `sort_by_cmp_wrappers` path hides cmp errors)
2. **`list.sort()` v0 arm silently no-ops mixed lists** —
   methods.rs:1945 `if all_int || all_str` — else the list is left
   unsorted, returns None, NO error: `[3,"a",1].sort()` → unchanged;
   CPython raises TypeError. Probed live.
3. **`key=None` crash** — `sorted(x, key=None)` and
   `list.sort(key=None)` fail with
   `TypeError: 'NoneType' object is not callable`
   (builtins.rs:2880 treats `Some(None)` as a key fn; builtins_modules
   :17634 `callable_name(None)` raises). CPython: key=None == no key.
   Also observed: `sort(reverse=False)` produced a corrupted frame
   (subsequent `print` "not callable") — flagged, not root-caused
   (methods::call_method carries no kwargs; kwargs arrive only on the
   pylist arm — the seam is suspect).
4. Minor: pylist-arm reverse does `ord.reverse()` + unconditional
   `keyed.reverse()` + `.into_iter().rev()` — the last two cancel;
   tie-stability under reverse currently matches CPython by accident of
   the cancellation. Fragile, deserves a comment/test when touched.

### Compare vs CPython + Rust SOTA

- CPython `listsort`: merge sort + binary insertion, galloping,
  `unsafe_long_compare`/`unsafe_string_compare` fast paths that bypass
  PyObject_RichCompare on homogeneous int/str runs. Our v0
  homogeneous arm IS this strategy (minus galloping).
- Rust SOTA: `sort_unstable` = pdqsort (unstable — unusable for
  Python semantics), stable `sort_by` = driftsort — what we use.
  Hashbrown irrelevant here.
- Verdict driver: the gap is (a) the Int interpreter pathology, (b)
  per-compare user-slot dispatch on the generic path, (c) the silent
  correctness bugs which any "fix" must not preserve.

### Verdict: FIXABLE

- Fix A (correctness, small): map comparator Err to the real
  TypeError; make the v0 arm raise on mixed lists; accept key=None.
  Est: 0.5 day + differential tests.
- Fix B (perf): extend the homogeneous pre-scan to the `sorted()` path
  (or route sorted() through the same fast arm): pack to raw i64/str
  comparators, no user dispatch, fall back generically. Est gain:
  int sorted() 20,000x → single-digit-x of CPython (assuming the
  Int pathology is fixed/ruled out separately); str/float 18x → 2-4x.
  Est: 2-4 days incl. differential tests.
- Fix C (investigate): profile the 35µs/element Int-loop pathology —
  it gates A+B's numbers and P09 entirely. Est: 1-2 days.

---

## 2. dict (P09)

### Actual state

`crates/ap-obj/src/dict.rs` (1061 lines) — a first-hand CPython
compact-dict transcription:

- Sparse **index table** (`Vec<isize>`, DKIX_EMPTY/-1, DKIX_DUMMY/-2
  tombstones) + dense **entries Vec** in insertion order
  (dict.rs:110-123). Insertion order survives deletes; deleted-then-
  reinserted keys append (header doc lines 13-16, probed against 3.14
  per the doc).
- Probe recurrence is CPython's EXACT one: `i = hash & mask; perturb
  >>= 5; i = (i*5 + perturb + 1) & mask` (dict.rs:155-182).
- Resize at 2/3 load (`(used+dummies+1)*3 > indices.len()*2`,
  dict.rs:188) to `used*3` rounded up to power of two, purging
  dummies (dict.rs:208-231) — mirrors `GROWTH_RATE`/`USABLE_FRACTION`.
- Key collapse `{1:'a', 1.0:'b'}` → `{1:'b'}` probed LIVE this
  session — matches CPython.
- Concurrency: per-dict `Mutex<DictState>` (02.09b decision 2,
  dict.rs:67-85); `set()` uses try_lock + 50µs-sleep retry
  (dict.rs:240-245); mutation epoch AtomicU64 for the GC seed cache.
- `items()` clones EVERY key+value pair into a fresh Vec
  (dict.rs:440-447); `foreach()` borrow-guarded variant exists
  (dict.rs:456+) but few callers use it.

### Hash-cache story (review checklist — verified)

- **str hashes ARE cached and the cache survives lookups**:
  `PyString.cached_hash: AtomicI64` (value.rs:220), 0 = "not computed"
  sentinel, `-1 → -2` normalized BEFORE caching so the sentinel domain
  is provably collision-free (value.rs:310-323 proof comment). Cache
  read/store at value.rs:325-349, Relaxed ordering; clones copy the
  cached value (value.rs:360-362). `dict.get(&Value::Str)` →
  `hash_of` → `sv.py_hash()` → cache hit on every lookup after the
  first. This is CPython-parity (unicode hashes are memoized in the
  str object there too).
- int hashes are recomputed per lookup (`hash_i64`, a modulo — cheap);
  CPython also recomputes. Frozenset/tuple hashes recompute per call —
  a potential (minor) cache opportunity, not measured.
- `get_str`/`contains_str` fast paths probe WITHOUT minting a
  Gc<PyString> (dict.rs:328-350) — good.

### Measured gap attribution

- dict.build str @20k = 5.2x CPython; dict.lookups str @20k = 135x.
  The hash table itself is not the story: per-op cost is
  interpreter-loop + bound-method dispatch — every `d.get(k)` in the
  probe minted a BoundMethod, went through `methods::call_method`
  (methods.rs:2127-2131), cloned the value (`d.get(...).unwrap_or`)
  per hit. The int-keyed rows (1,900x) are the SAME Int-loop pathology
  as P08 (listcomp_int_20k = 0.709 s vs listcomp CPython 0.13 ms —
  the loop, not the dict).
- `bench_report.json` P09 rows: all ERROR/TIMEOUT — explained by the
  same pathology at 1M scale (slow-not-hung; classification matches
  baseline-notes.md's own note).

### Compare vs CPython + Rust SOTA

- CPython: open addressing with the identical recurrence + cached str
  hashes. We match structurally, entry-for-entry.
- Rust SOTA: hashbrown — SIMD-group probing (16-way control bytes),
  Swiss tables. Structurally different from CPython's compact layout,
  and adopting it would BREAK insertion-order-compact semantics unless
  layered (hashbrown + side order vec ≈ what index table already does
  for us). Not worth it while the measured gap is loop/dispatch-bound.

### Verdict: ALREADY-PARITY (structure) — the measured gap is NOT the dict

- The data structure is CPython-exact; nothing to fix at the primitive
  level. The P09 gap belongs to (1) the Int interpreter pathology
  (Fix C above) and (2) per-call method dispatch on `d.get`/`d[k]`
  (an interp fast-path matter: inline the dict-getitem marker route).
  Record both as cross-primitive interpreter work, not dict work.
- Optional small win: route dict iteration-heavy callers through
  `foreach()` instead of `items()` (clone-free) where re-entrancy
  allows.

---

## 3. json (P10)

### Actual state — NATIVE RUST, not vendored pure-Python (review checklist: grounded in the import route)

The leaf spec asked to check for a vendored pure-python `json.py`.
Verified it is NOT:

- Import route: `json` is registered as a NATIVE module —
  `builtins_modules.rs:854` `("json", build_json)` in the native
  module table; `build_json` (builtins_modules.rs:7361-7405) mints the
  module namespace with native fn markers for
  dump/dumps/load/loads/detect_encoding + JSONDecodeError/JSONEncoder/
  JSONDecoder type surfaces.
- Calls route through `call_json` (builtins_modules.rs:7407-7484) →
  `json_loads` (:7786) / `json_dumps` (:7843).
- `json.__file__` is None (probed live); no `json/` directory exists
  under `vendored_stdlib/` (only an unrelated `_sysconfig_vars__*.json`
  data file); `embedded_stdlib/data.rs` contains zero json entries
  (grep = 0). There is no fallback path that could reach a python
  json.py.

Implementation shape:
- **Parser** (`JsonParser`, builtins_modules.rs:7555-7783):
  hand-written recursive-descent over `&[u8]` + `&str`, byte-at-a-time;
  parse_string walks char-by-char with a growing `String` and per-escape
  pushes; `\uXXXX` via `from_str_radix` on 4-byte slices with surrogate
  pairs; parse_float/object_pairs_hook name-dispatch hooks supported.
- **Encoder** (`json_dumps` :7843 + `dump_value` :7921): recursive
  tree-walk; `dump_string` (:8101) per-char with `format!("\\u{:04x}")`
  per escape; allow_nan pre-scan (:7852-7891); separators/ensure_ascii/
  sort_keys/indent kwargs honored; `sort_keys` sorts a cloned items vec
  with **type_name() as the key for non-str keys** (:7960-7971) —
  CPython raises TypeError there (or skipkeys-skips); minor divergence
  worth a row in NEXT-PYDIFF.
- Error surface: `json_error2` reproduces CPython's
  "msg: line L column C (char P)" texts + LAST_JSON_ERR side table for
  e.pos/e.lineno attrs.

### Measured

dumps ≈ CPython parity (0.91-1.02x — PASS per master's "beat CPython
as shipped" target, marginal). loads = **4.3-4.5x slower than
CPython**. CPython loads is the C `_json` scanner (bulk string scan +
memo dict); ours is byte-at-a-time Rust with per-char String growth and
per-slice from_str_radix. serde_json-class (bulk memchr scans, SIMD-ish
dispatch, pre-sized buffers) typically runs 5-10x faster than
CPython's C scanner — the headroom is real and reachable in Rust.

### The native-json hot-path leaf sketch (json is native, so this is an optimization leaf, not a port leaf)

Goal: loads ≥ CPython (target 2-4x CPython), dumps ≥1.5x CPython,
byte-exact output preserved. No new dependency REQUIRED; serde_json
adoption is a separate user decision (see below).

1. **String scanning (loads, biggest win)**: replace the per-char
   string loop with `memchr`/`memchr::iter` for the closing quote and
   backslash; bulk-copy escape-free spans (`out.push_str(&doc[start..q])`)
   instead of per-char pushes. Est: 2-3x on string-heavy payloads.
2. **Number parsing**: scan the numeric span with a byte classifier,
   then one `str::parse::<f64>()` (Rust's grisu — already fast) or the
   existing Int fast path for integer spans; avoid `from_str_radix`
   on 4-byte `\u` slices by a 16-entry nibble table.
3. **Encoder**: pre-size the output String (sum of child estimates),
   replace `format!` escapes in `dump_string` with a static escape
   table + `push_str`, and write escape-free spans in bulk. Est:
   dumps 0.91x → 1.5-2.5x.
4. **Structure**: keep the recursive-descent skeleton (it is what
   guarantees CPython-exact error texts and hooks) — do NOT swap for
   serde_json's parser; the differential corpus (s008/s009 +
   JSONDecodeError texts + parse_float/object_pairs_hook) is the gate.
5. **sort_keys divergence**: decide + fix (TypeError on non-str keys
   vs CPython; skipkeys unsupported) while in the file.
6. Effort estimate: 2-4 days incl. re-running the json differential
   corpus byte-exact.
7. **New-dependency note (user decision needed)**: adopting
   `serde_json` (or `memchr` alone) — memchr is a tiny, Apache/MIT,
   broadly-trusted dep and unlocks (1); serde_json as an actual
   implementation would REPLACE the parser and forfeit the hand-built
   CPython error-text surface — recommended NO for serde_json, YES
   (cheap) for memchr.

### Verdict

- dumps: ALREADY-PARITY (marginal 0.91-1.02x).
- loads: FIXABLE — algorithmic (scan-shape, not asymptotic); est 3-4x
  from the sketch, target 2-4x CPython; serde-class 5-10x only with a
  full rewrite = NOT recommended (error-text/hook fidelity risk).

---

## Verdict summary

| primitive | verdict | est gain | est effort |
|-----------|---------|----------|-----------|
| P08 sorted()/list.sort generic path | FIXABLE | 20,000x → <5x (int, contingent on Fix C); str/float 18x → 2-4x | 2-4 days + 0.5 day correctness |
| P08 list.sort() homogeneous fast path | ALREADY-PARITY (5.2x, close; galloping optional) | marginal | — |
| P09 dict structure | ALREADY-PARITY | — (gap is interpreter-loop + method dispatch, cross-primitive) | — |
| P10 json.dumps | ALREADY-PARITY (marginal) | 1.5-2.5x available via escape-table | 0.5-1 day |
| P10 json.loads | FIXABLE | 4.5x-slower → 2-4x faster than CPython | 2-4 days |

New-dep proposals (user decision needed): `memchr` (recommended, tiny);
`serde_json` (recommended against — replaces the CPython-exact error
surface).

Counts: parity 3 (sort-fast-path, dict structure, json.dumps) /
fixable 2 (sort generic path, json.loads) / not-worth 0.

## Honest-stop ledger

- Int-loop ~35µs/element pathology: root NOT identified (two profiling
  attempts spent: presortedness-invariance probe + float/str cross-type
  probe). Filed as Fix C with suspects listed.
- `list.sort(reverse=False)` frame-corruption oddity: observed once,
  not root-caused (attempt budget).
- P10 PASS claim in baseline-notes.md vs zero P10 rows in
  bench_report.json: flagged for orchestrator reconciliation; hand
  measurements recorded above instead.
- random.Random missing on the native random module (P08 probe
  crash): real surface gap, owned by neither this leaf nor 00-harness —
  flag to master.
