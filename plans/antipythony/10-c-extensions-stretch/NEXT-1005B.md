# NEXT-1005B — Leaf session record (S1 band 2: object-protocol C-API)

Date: 2026-11-22. Worktree: `~/.git/worktrees/leaf-1005b` (branch
`tier1-1005b`, off main `1cd873c`). CARGO_TARGET_DIR=target-1005b.

## Commits (branch tier1-1005b)

1. `ff666d3` — ap-ce exports_objproto (NEW): the band's CPython-spelled
   exports + the registered ObjProtoVTable seam + header declarations
   (`// --- leaf 1005b ---` marker at include/apx_python.h end).
2. `a82cd4c` — ap-run ce_objproto (NEW): the vtable impls + the two
   additive interp shims (`compare_values_pub`, `class_protocol_builtin_pub`)
   + the 3-line ce_bridge install hook.
3. `9b08af9` — proof extension (tests/ce-s1/objproto.c + driver +
   compat header) + 8 new ap-ce unit gates.

## Members landed (~26 exports)

Attr: Get/Set/DelAttr, Get/Set/DelAttrString, Get/SetOptionalAttr{,String}
(6+2), Length, LengthHint, RichCompare, RichCompareBool, IsTrue, Not,
GetIter, IterNext, Hash, Dir, apx_PyIter_Check, PyLong_FromSsize_t,
apx_DisplayTypeName. PyObject_Call family regression-pinned (10.04).

## Architecture (the layering decision)

ap-ce cannot depend on ap-run, so — 10.04's PycallFn pattern again —
the band rides a REGISTERED VTABLE (`install_objproto_vtable`), installed
by ap-run's `ce_objproto::ensure_objproto_installed()` from
`ensure_pycall_installed()` (3 hook lines; dispatch bodies untouched).
Every impl defers to the runtime's OWN protocol layer
(attr_get_opt / attr_set/del_protocol / class_protocol_builtin /
compare_values_pub / builtin_iter / pull_iter_pub / gen_pull_public /
py_hash) so a C extension sees byte-identical behavior with the Python-
level expression. GetIter/IterNext bridge the EXISTING iterator registry
(builtin_iter => register_shared_iter lifecycle; generators keep lazy
ids) — NO second registry. IterNext returns NULL with the error state
guaranteed CLEAR at exhaustion (StopIteration Flows mapped to exhaustion;
PyO3 loop contract).

## Gate-found bugs (cost honesty)

1. **Attr-miss dialect split**: CPython's READ miss ("'int' object has
   no attribute 'x'") vs SET/DELETE miss (same + "and no __dict__ for
   setting new attributes") — one text served both, R9/R15/A9 diverged.
   Fixed: two dialect fns; the set path also REJECTS builtin no-__dict__
   writes at the C surface (the runtime's attr_set_protocol no-ops them
   for enum member creation — that internal path untouched).
2. **length_hint order wrong**: I implemented hint-first; CPython's
   abstract.c is LEN-first when the type has len (probed: LenFirst
   hint=2 len=9 => 9), hint second with TypeError-from-call => default
   and other errors PROPAGATING (ValueError('boom') survives). Read the
   3.12 source to settle it.
3. **RichCompareBool has an identity shortcut** RichCompare lacks
   (probed: x==x answers 1/0 without dispatch; rich-compare proper
   always dispatches — nan != nan goes through eq).
4. **op-spelled compare errors**: values_cmp hardcodes '<' (the
   executor only chains Lt/Gt); the C surface needs '<='/'>'/'>='.
   compare_values_pub re-spells (Python callers unaffected). First
   splice attempt produced `'<=''` (off-by-one) — fixed with
   strip_prefix.
5. **`__hash__ = None` class**: runtime calls None ("'NoneType' object
   is not callable"); CPython says "unhashable type: 'Unhashable'".
   The bridge checks the slot before dispatching.
6. **CPython PyIter_Next has NO type check** (bpo-19005: caller
   precondition, segfaults by design on this Homebrew build). The
   proof extension guards with PyIter_Check (apx_PyIter_Check seam);
   I8 pins the guard's TypeError.

## Documented gaps (pre-existing runtime layer, NOT bridge bugs — disclosed)

- **dir() on builtin-type instances** ("abc"/5) is empty at the PYTHON
  level too (dir_surface's per-type tables only cover the `int` interned
  TYPE arm: dir(int)=52 vs CPython 74; dir(str)=0 vs 81). The driver
  pins instance/module dir + sortedness instead.
- **dir_c(SomeClass)**: a non-spec user-class handle pins to a Str
  through state.rs from_value's spec reverse-lookup — dir_c(instance)
  is the working surface.
- **iterator __length_hint__**: CPython's list_iterator answers the
  remaining count; the registry's Seq state isn't exposed to the C band
  yet (needs a registered ap-run export). The driver pins a generator
  (hint-less both sides) through the default fallback.

## Gates (executed, not claimed)

- **G1** `cargo test -p ap-ce`: 13 passed / 0 failed (5 baseline + 8 new).
- **G2** objproto.c + objproto_driver.py vs /opt/homebrew/bin/python3.14
  (3.14.7): stdout diff EMPTY, stderr diff EMPTY, rc 0/0. 73 pins.
- **G3** `cargo test -p ap-cli --test stdlib_modules_differential`:
  48 passed / 0 failed.
- Extra: build_ext_gate 4/4 (geom + spike byte-exact through the verb);
  callback.c 10.04 regression re-run byte-identical both arms.

## Sibling safety

Touched: crates/ap-ce/{exports_objproto.rs NEW, lib.rs (+1 line),
tests/ce_s1_units.rs (+8 tests)}, crates/ap-run/{ce_objproto.rs NEW,
lib.rs (+1 line), interp.rs (2 additive pub shims + eval_compare
delegating 3-line wrapper — protocol body untouched), ce_bridge.rs
(3 hook lines)}, include/apx_python.h (band declarations, end-marker),
tests/ce-s1/{objproto.c, objproto_driver.py, apx_cpython_compat.h NEW}.
cargo fmt reformatted 6 sibling files mid-session — REVERTED before
commit (verified: `git show --stat ff666d3 a82cd4c 9b08af9` shows only
the listed files). exports.rs untouched (PyLong_FromSsize_t landed in
exports_objproto.rs to avoid editing the frozen file).
