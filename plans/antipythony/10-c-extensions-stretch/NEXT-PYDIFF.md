# NEXT-PYDIFF — the pydantic 110-member CPython-arm byte differential

Date: 2026-11-23. Worktree: `~/.git/worktrees/leaf-pydiff` (branch
`tier1-pydiff`, off main `1722168`). Target dir: `target-pydiff`
(CARGO_BUILD_JOBS=4; one heavy cargo lane held).

## VERDICT: RECORDED — both arms compile and run; every member either
BYTE-EXACT or a recorded divergence with exact bytes.

CPython 3.14.7 arm (cc -DAPX_CPYTHON_BUILD + Python.h through
`apx_cpython_compat.h`): rc=0, 106 stdout lines, md5 b5bc1490…,
3-run deterministic. stderr: 1 line
(`Exception ignored in: ValueError('unraisable-probe')` — the
WriteUnraisable posture line, see D-below).

antipythony arm (`antipythony build-ext` + `antipythony run`): rc=0,
106 stdout lines, md5 4fde4cf6…, 3-run deterministic, empty stderr.

Diff: **62 diverging stdout lines** (31 tag pairs of 106) — findings,
not failures; NO engine code touched (crates/ untouched, verified by
git log).

## What landed

1. `include/apx_python.h` — arm-neutral aliases band: `apx_unicode_writer`
   typedef, `apx_iter_next_impl` (maps the band's has_next out-slot onto
   CPython's (item, rc) contract for the driver), `apx_unicode_maxchar`,
   `apx_interp_id`, `apx_empty_tuple_const`, 15 `apx_*_type` token
   accessors, `apx_modinit`. Commit 8725ef6.
2. `tests/ce-s1/apx_cpython_compat.h` — the SAME alias names defined on
   the CPython side (PyExc_ + token macro neutralizations, PyUnicodeWriter*
   handle mapping, InterpreterState spellings, Py_CONSTANT_* passthrough).
   One source compiles both ways. Commit 8725ef6.
3. `tests/ce-s1/pydantic_diff.c` + `pydantic_diff_driver.py` +
   `expected/pydantic_diff_cpython314_stdout.txt` — the differential
   source (13 probe fns over every member family), driver, and the
   CPython reference. Commit 43222f7.

## Gate (executed, not claimed)

- `cargo test -p ap-ce` (target-pydiff): **36 passed / 0 failed**
  (baseline held).
- `cargo test -p ap-cli --test stdlib_modules_differential`: **48
  passed / 0 failed** (baseline held).
- Both-arm determinism: 3 runs each, md5-stable.
- crates/ untouched: `git log --stat` shows only include/ + tests/ce-s1/.

## DIVERGENCES (the leaf's deliverable; each with exact bytes)

Notation: C = CPython line, A = antipythony line.

1. **M11 GenericGetDict-on-dict**: C `M11 <null>` vs A `M11 {}`.
   `PyObject_Repr` of the GenericGetDict answer: CPython's dict-backed
   __dict__ answers NULL from the C probe path (no dict payload on a
   plain dict instance via GenericGetDict); ours answers the real empty
   dict handle. Deeper: GenericGetDict identity/shape.
2. **C3 complex error contract**: C `C3 0` vs A `C3 1`.
   `PyComplex_RealAsDouble(Py_None)`: CPython raises TypeError (returns
   0.0 WITH error set → `== 0.0 && PyErr_Occurred()` is 1 on CPython? NO
   — CPython's check returned 0 because PyErr_Occurred() was NULL: the
   real CPython raises TypeError only for non-complex TYPES (None IS
   handled by `__complex__` absence → TypeError... measured: 0). Ours:
   err_set fires (1). The member exists both arms; the error contract
   differs. (See the raw C2 line too: C `(1.5-2.5j)` vs A
   `<complex(1.5, -2.5) object at 0x…>` — the KNOWN repr-pin posture,
   arm-shaped and excluded from the pins.)
3. **U5 PyUnicode_New fill bytes**: C `U5 '{7}'` vs A `U5 '\x00\x00\x00'`.
   CPython's PyUnicode_New(3, 0x7f) leaves UNINITIALIZED memory (the
   measured run happened to show stale `{7}` bytes — undefined); ours
   NUL-fills deterministically. Divergence recorded; CPython's own line
   is not a contract (uninitialized memory).
4. **U7 AsEncodedString return type**: C `U7 b'caf\xc3\xa9'` vs
   A `U7 'café'`. CPython returns BYTES; the band returns a Str handle
   carrying the encoded text. EXACT: our AsEncodedString answers str,
   CPython answers bytes.
5. **I1 PyImport_Import("json")**: C `I1 1` vs A `I1 0` — the runtime
   import of `json` FAILS on the ce-module import seam (no runtime
   interp attached to the C context at that point / json not resolvable
   through mapping_import_module from the ce context). Import-via-C
   member: deeper-seam finding.
6. **I3 interpreter id**: C `I3 -1` vs A `I3 1`. CPython's
   `PyInterpreterState_GetID(PyInterpreterState_Get())` under the
   extension-call context answered -1 (no interpreter state reachable
   through Get() in that frame); ours pins 1 (NEXT-1006 contract).
   NOTE the pinned-1 claim vs the probe: both arms' answers are
   context-dependent; recorded with bytes.
7. **E1 PyException_GetCause fresh exception**: C `E1 0` vs A `E1 1`.
   CPython: a freshly-raised exception has cause NULL (not None);
   ours answers Py_None. NULL-vs-None contract divergence.
8. **E4 SetTraceback return**: C `E4 0` vs A `E4 1`. CPython's
   PyException_SetTraceback(exc, NULL) returns -1 (a live exception
   must keep a tb slot; setting NULL on a fresh exc fails); ours
   accept-and-forget returns 0.
9. **T1-* token Getname (13 lines) + T9/T10**: C answers the REAL type
   name per token (`T1-2 bool bool`, `T9 'int'`); ours answers
   `object` for every token except `type` (`T1-2 bool object`,
   `T9 'object'`). Root cause (read, not fixed): the band's tokens are
   Str handles (`exports_pydantic.rs` apx_type! macro → CePayload::Str),
   so `PyType_GetName`'s CePayload::Type arm misses and falls to the
   "object" fallback. The token members are IDENTITIY-ONLY on S1 —
   this differential MEASURES that posture for the first time.
   Affected members: PyType_GetName/GetQualName over 14/15 tokens (the
   `type` token itself answers correctly both arms: `T1-0 type type` vs
   `type object` — the NAME is right, the qualifier differs).
10. **T12 Callable_Check on a type token**: C `T12 1 0` vs A `T12 0 0`.
    CPython: PyCallable_Check(&PyLong_Type) is 1 (types are callable);
    ours: Str-handle token → 0. Same root cause as #9.
11. **T14 GenericAlloc on a type token**: C `T14 1` vs A `T14 0`.
    CPython allocates a module instance; ours: the token is not a
    FromSpec type handle → err "not a FromSpec type handle" → NULL.
    Same root cause as #9 (token representation).
12. **X3/X5/X7 PyIter_NextItem rc**: C `X3 0` vs A `X3 1` (and X5, X7).
    The compat alias maps CPython's (item-out, rc) onto the driver's
    rc==0 convention; the antipythony band's own has_next out-slot
    semantics flow through apx_iter_next_impl and answer 1 where
    CPython's rc is 0. The ITEMS themselves are byte-identical (X4/X6
    `4`/`5` both arms; exhaustion NULL both arms). Member-exists +
    item-correct; the RC CONVENTION diverges (band's two-out-param
    shape vs CPython 3.14's item-out shape — the header declared the
    has_next signature, CPython 3.14 changed it).
13. **X8 Mapping_Items**: C `X8 [('k', 1)]` vs A `X8 <null>`. The band's
    Mapping_Items attr-routes `items` off the value; the dict-value arm
    misses (err path) → NULL. Mapping_Items member: deeper finding.
14. **X9 PyCMethod_New**: C `X9 1` vs A `X9 0` — RECORDED POSTURE (the
    band fails loud: "Def-class methods are an S2 member"), confirmed
    by measurement. CPython builds the method object.
15. **B3/B4 (bytearray error contract)**: BYTE-EXACT after the driver
    fix (Size on non-bytearray fails both arms). Earlier raw-value
    probes on CPython were UNDEFINED (uninitialized read; the value
    changed run-to-run) — the driver now pins the error contract, which
    is byte-identical.

## Byte-exact bands (the rest of the 106 lines)

M1–M10/M12–M16 (mapping/sequence/isinstance surfaces incl. error
dialects), N1–N9 (number band: Remainder/TrueDivide/Index/FromInt32/
FromUInt32/From-AsUnsignedLongLong overflow), S1–S7 (sets: New/Add/
Contains/Frozen/New-from-list/New-non-iterable error; set repr order
is hash-order — S3 repr was measured UNSTABLE on antipythony across
runs ({1,2} vs {2,1}) and pinned as SIZE instead; recorded as a
separate known ap-obj finding, not a new one), C1 (complex numeric
accessors), U1–U4/U6/U8/U9 (writer round-trip incl. WriteChar,
EqualToUTF8AndSize, InternInPlace, unknown-encoding LookupError,
UnicodeDecodeError_Create), B1/B2 (bytes round-trips), MO1/MO2
(module name projection), I2/I4/I5/I6 (missing-module ImportError,
IsInitialized/Finalizing, GetConstant NONE identity, EMPTY_TUPLE value),
E2/E3/E5–E7 (cause round-trip, traceback accessor NULL, WarnEx
suppressed posture, WriteUnraisable clears, PrintEx-exists posture),
T11 (GetModuleName 'builtins' both), T13 (Freeze accept), X1/X2
(Capsule_Import fails-loud ImportError posture CONFIRMED byte-exact
with CPython's missing-capsule ImportError; PyObject_Free), Z0–Z9 (ALL
TEN PyExc_ tokens: SetString + Occurred round-trip byte-exact), W1
(number writer round-trip).

## stderr

CPython prints the WriteUnraisable posture line
(`Exception ignored in: ValueError('unraisable-probe')`); antipythony's
WriteUnraisable clears silently (empty stderr). Recorded; stdout is
the diffed surface per the established pattern.

## Honest gaps

- The `type`/token representation (finding #9/#10/#11, 17 diverging
  lines) is the LARGEST cluster — a fix would give tokens a
  CePayload::Type arm in the band; that is ENGINE work (filed as the
  named follow-up; this leaf records only).
- PyIter_NextItem's rc convention (#12) wants the header declaration
  aligned to CPython 3.14's item-out shape (header change + band fix;
  filed with #9).
- PyErr_PrintEx could not be CALLED on the CPython arm (in-extension
  invocation segfaults CPython's sys.excepthook chain — by-design
  hard-exit semantics); pinned by existence both arms instead.
- The cp314 wheel itself is still not loaded (rebuild-required is the
  locked policy); this differential proves the BAND's members against
  real CPython, same rebuild-simulated scope as NEXT-PYDANTIC-PROBE.

## Machine policy

One cargo lane at a time; CARGO_BUILD_JOBS=4; worktree+logs under
~/.git/worktrees/leaf-pydiff. Commits: 8725ef6 (headers), 43222f7
(diff source + driver + reference). Explicit pathspecs throughout; no
`git add -A`.
