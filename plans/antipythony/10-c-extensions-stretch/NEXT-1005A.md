# NEXT-1005A — Leaf 1005a session record (S1 container band)

Date: 2026-11-22. Worktree: `~/.git/worktrees/leaf-1005a` (branch
`tier1-1005a`, off main `1cd873c`). Target dir:
`target-1005a` (own lane; `target` symlink staged to leaf-1004's
`target-0304c` for the pre-build phase, replaced by real builds).

## What landed (branch tier1-1005a)

1. `b4f560b` — `crates/ap-ce/src/exports_containers.rs` (NEW, ~1270
   lines): 59 unprefixed CPython-spelled exports over the S0 handle
   layer. PyDict: GetItem, GetItemWithError, GetItemRef,
   GetItemString, GetItemStringRef, SetItem, DelItem, DelItemString,
   Clear, Contains, ContainsString, Pop, PopString, SetDefault,
   SetDefaultRef, Keys, Values, Items, Size, Copy, Merge, Update,
   MergeFromSeq2, Next. PyList: Size, GetItem, GetItemRef, SetItem,
   GetSlice, SetSlice, Insert, Extend, Clear, Sort, Reverse, AsTuple.
   PyTuple: GetSlice + apx_PyTuple_Pack (array form; varargs route in
   apx_shim.c `apx_PyTuple_PackV` + header macro). Plus
   PyLong_FromSsize_t. Header: clearly-marked `--- leaf 1005a ---`
   append block in include/apx_python.h.
2. `db7f458` — tests/ce-s1/containers.c (~90 C probe fns) +
   containers_driver.py (124-line byte-compared output) +
   expected/containers_cpython314_stdout.txt (pinned CPython 3.14.7
   reference run).
3. `3adc6d4` — differential dialect corrections (round 2, all
   gate-found; see findings).

## Measured CPython 3.14.7 dialect table (the leaf's core deliverable)

The container internals raise **SystemError
"Objects/<file>.c:<line>: bad argument to internal function"** with
CPython's EXACT object-file line numbers on wrong-typed receivers —
NOT TypeErrors. Pinned lines (unit gate `dialect_table` +
`dialect_table()` accessor): listobject.c 306/388/419/453/507/717/
1026/1498/3170/3204/3219; dictobject.c 2524 (GetItemWithError)/2468
(GetItemRef)/2508 (GetItemStringRef)/2722 (SetItem)/2854
(DelItem)/2798 (GetItem)/3561/3610/3658 (Keys/Values/Items)/3767
(MergeFromSeq2)/4003 (Merge)/4231 (Copy)/4248 (Size)/4421
(SetDefault)/3166 (Pop); tupleobject.c 479 (GetSlice). Measured by
throwaway probe extensions against the host CPython (scratch-1005a/).

Other dialects measured and pinned:
- KeyError arg = the key's REPR (exc_hier renders str(e) = repr(arg)).
- Unhashable dict key: "cannot use 'list' as a dict key (unhashable
  type: 'list')" on SetItem/DelItem/Contains/Pop/SetDefault/
  GetItemWithError/GetItemRef; `PyDict_GetItem` MASKS it (NULL + NO
  error state — the C contract; 3.14 also prints an unraisable
  stderr notice we do not reproduce).
- Merge/Update non-mapping: AttributeError "'<t>' object has no
  attribute 'keys'" (the keys()-chain head, incl. str/int sources).
- MergeFromSeq2: outer non-iterable -> "'<t>' object is not
  iterable"; inner non-iterable item -> BARE "object is not
  iterable"; bad arity -> ValueError "dictionary update sequence
  element #N has length L; 2 is required"; STR items are
  PySequences ("ab" merges ('a','b')); dict SOURCE iterates KEYS
  (not items()).
- PyList_Extend non-iterable: "'<t>' object is not iterable".
- Mixed Sort: "'<' not supported between instances of 'str' and
  'int'" (order follows the compared pair — values_cmp's text).
- IndexError texts: "list index out of range" / "list assignment
  index out of range" / "tuple index out of range" (C API does NOT
  wrap negative indexes).

## GATE-FOUND BEHAVIORAL FACTS (cost honesty, each cost a debug cycle)

1. **CPython 3.14 refuses PyTuple_SetItem on rc>1 tuples** (measured:
   a Python-passed tuple arrives rc=4 -> SystemError
   tupleobject.c:123 on BOTH runtimes). Extensions must
   construct-and-fill (PyTuple_New then SetItem) — the proof probe
   does exactly that. This means the S0 PyTuple_SetItem
   "grow-for-sparse-writes" divergence is unreachable from real
   extension code (softens finding 3 below).
2. **PyDict_Contains / PyDict_GetItemRef(null result) SEGFAULT on
   non-dict receivers in real CPython 3.14** — probes removed from
   the differential (no oracle exists; my exports return the
   line-pinned SystemError instead — strictly safer, unobservable in
   correct programs).
3. **Per-call-mint handle aliasing**: in-place list mutation through
   a handle does not propagate back to the original runtime list
   object (CePayload::List is a Vec<CeHandle> snapshot; dicts work
   because CePayload::Dict carries the Gc<PyDict>). Fix within my
   scope: mutators return the handle; the driver threads returns
   (legitimate C-API usage). RECORDED for the state.rs owner: making
   CePayload::List carry Gc<PyList> (the Dict shape) would remove the
   snapshot semantics — a sibling-file change outside this leaf.

## Findings for sibling owners (exports.rs is off-limits to this leaf)

- S0 `PyTuple_Size/GetItem/SetItem` (exports.rs) diverge from 3.14:
  wrong-receiver text "expected tuple, not '<t>'" vs the measured
  line-pinned SystemError; GetItem ACCEPTS negative indexes (CPython
  does not); SetItem grows the tuple for i>=len. Behavior identical
  on valid inputs; the differential exercises valid-input + OOB
  GetItem (matching) only.
- S0 `PyList_Append` (exports.rs) wrong-receiver text is
  "expected list, not '<t>'" vs measured listobject.c:542 SystemError.
- These are one-line bad_internal()-style fixes in exports.rs when
  its owner picks them up; the dialect table is in
  exports_containers::dialect_table().

## Gates (executed, not claimed)

- `cargo test -p ap-ce` (CARGO_TARGET_DIR=target-1005a): **19 passed /
  0 failed** (5 baseline + 14 new container-band unit gates).
- Byte-exact differential: containers.c built via
  `antipythony build-ext` (own worktree binary), driver run under the
  built binary AND /opt/homebrew/bin/python3.14 (same source,
  APX_CPYTHON_BUILD): **stdout + stderr diff EMPTY, RC=0 both**
  (124 lines).
- `cargo test -p ap-cli --test stdlib_modules_differential`: **48
  passed / 0 failed** (the task's "stdlib 48/48" gate — the suite's
  actual name).
- Machine policy held: one cargo lane at a time (waited out sibling
  `cargo test -p ap-cli -p ap-conform` + `cargo test -p ap-run`
  lanes before heavy runs); CARGO_BUILD_JOBS=4; all artifacts under
  ~/.git/worktrees/leaf-1005a (scratch dir inside, never /tmp).

## Sibling safety

Touched ONLY owned files: crates/ap-ce/src/exports_containers.rs (NEW),
crates/ap-ce/src/lib.rs (one `pub mod` line), crates/ap-ce/c/apx_shim.c
(apx_PyTuple_PackV helper — same leaf-adjacent C shim, additive),
include/apx_python.h (marked append), tests/ce-s1/{containers.c,
containers_driver.py, expected/}, crates/ap-ce/tests/. OFF-LIMITS files
(exports.rs/api.rs/slots.rs) touched only by incidental `cargo fmt`
whitespace drift during formatting — REVERTED before every commit
(verified: `git show --stat` shows owned paths only).

## Honest gaps

- ap-run/ap-cli suites beyond stdlib_modules_differential not re-run
  (ce changes are additive exports; the OnceLock registries are
  no-ops until a ce import; stdlib 48/48 covers the trunk path).
- Refcount probe asserts DELTAS + value identity, not absolute
  counts (impossible across layouts; PEP 703 posture — the spike
  probe pattern).
- Key rendering: ce_bridge wraps KeyError args as Str (double-quoted
  vs CPython's bare-key repr when the DRIVER prints str(e)); the
  differential prints class-only for KeyError — the arg-side
  rendering divergence is an ap-run/ce_bridge finding (off-limits
  here), tracked for its owner.
- ~40 additional abstract-layer container members (PySequence_Fast*,
  PyObject_GetIter family) are the NEXT band; this leaf scoped to
  the concrete PyDict_/PyList_/PyTuple_ surface per the feasibility
  note's "Container/long/unicode surface completion" first tranche.
