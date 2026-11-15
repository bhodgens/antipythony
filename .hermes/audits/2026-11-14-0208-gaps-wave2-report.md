# 02.08 gaps wave 2 (s016/s020/s023/s025/s026/s028/s030) — wave report

Branch: tier1-0208gaps2 (worktree ~/.git/worktrees/leaf-0208gaps2, base main=972c499e).
Binary under test: ALL gate numbers below produced by
`/Users/caimlas/.git/worktrees/leaf-0208gaps2/target-0208g2/release/antipythony`
(worktree-only builds; the binary has the worktree vendored_stdlib path baked in,
verified via `strings`). CPython reference: /opt/homebrew/bin/python3.14 (3.14.7).
Corpus: worktree tests/corpus/stdlib (byte-identical to main's; diff -rq clean).

## Result: 4 of 7 gaps closed byte-exact (stdout+stderr+rc), slice 45/48 full-triple

| gap | status | commit(s) |
| --- | --- | --- |
| s023 ast | CLOSED FULL-EXACT | b266e12d |
| s025 dataclasses | CLOSED FULL-EXACT | 84893eea |
| s028 pprint/reprlib | CLOSED FULL-EXACT | 9dc23a46 |
| s030 copy.replace | CLOSED FULL-EXACT | 6445994c |
| s016 collections.abc warning | stdout+rc EXACT; stderr line residual | 310d35b3 |
| s020 inspect | PARTIAL (dis unblocks, deeper dis-model deferred) | 2171c5ce |
| s026 typing import | DEFERRED, pre-existing, root-cause data below | — |

Slice: 45/48 cases stdout+stderr+rc byte-exact (was 41/48 stdout-only).

## Closed-gap summaries
- s016: Interpreter.py_frames live python-frame chain; sys._getframe honors depth
  (over-walks clamp — CPython native frames invisible); check_isinstance consults
  type(C).__instancecheck__ for ALL subjects; ByteString native isinstance arms
  removed; class __class__ answers USER metaclass (was interned type).
- s023: vendored CPython 3.14 _ast_unparse.py pristine (+1 documented deviation:
  _Precedence.next via .value, no IntEnum arithmetic); _ast.py ASDL fixes
  (Module.type_ignores, Assign.type_comment, FunctionType, _attributes, optional
  fields default None, ctor kwarg DeprecationWarnings); bridge 0-based col_offsets,
  mode=eval Expression wrapper; %r dispatches user __repr__.
- s025: annotationlib.get_annotations consults __annotate__ (PEP 649) with resolved
  format; too-many-positional checked BEFORE binding; range-form arity messages;
  __qualname__ rewrite honored; list/tuple == elementwise full-eq recursion.
- s028: pprint._dispatch keyed by type NAME (runtime shares one __repr__ fn object
  across builtin types); py_lt tuple/tuple elementwise user dispatch (sort_dicts);
  interned builtin types answer __module__="builtins" (reprlib._lookup gate).

## Deferred (root-cause data)
1. s016 stderr line: DeprecationWarning attributed to line 14 (Stmt::Line marker =
   assignment start) vs CPython line 21 (expression line in a multi-line assign).
   Needs expression-level line markers in lowering (frontend-wide, Statement::Line
   granularity is per-statement).
2. s020 inspect: dunder-probe fix unblocked dis._get_code_object, but full
   getclosurevars/get_instructions needs real co_lines()/co_positions()/
   _varname_from_oparg surfaces on the synthetic <code> object (ArgResolver walks
   them). Also builtin signature lacks POSITIONAL_ONLY ("(x, /)") and eval_str
   annotation evaluation. Native `<code>` model = the deferred leaf.
3. s026 import typing HANG (pre-existing — reproduced with the wave-1 binary
   leaf-0208gaps/target-0208g/release/antipythony, NOT introduced this wave):
   macOS `sample` at hang: interpreter thread blocked in __psynch_mutexwait under
   ap_run::classes::exec_class_def <- exec_stmt <- run_loaded(typing module body)
   <- import_module. 0% CPU = lock-wait deadlock (not a spin). typing.py defines
   classes with metaclass=_ProtocolMeta; some Mutex taken in exec_class_def's
   metaclass path is re-entered or cross-ordered. Next bisect: instrument
   exec_class_def's metaclass-__new__ lock acquisitions.

## Regression gates (this tree)
- cargo test -p ap-run --release: ALL suites, 0 FAILED
- cargo test -p ap-obj --release: 0 FAILED
- cargo test -p ap-cli --release: packaging_differential 7/7, tier_mode 3/3,
  osr_gate 2/2, ast_getattr_fence 2/2 (after building the probe bins the suites
  expect under ../../target/{release,debug}); stdlib_modules_differential
  47/48 — t2_union_introspection hangs on the SAME pre-existing s026 typing
  import (verified hanging on the wave-1 binary too).
