# 2026-11-07: PyDict.update self-deadlock (blocks gc/metaclass batteries)

**Symptom.** wave3 gate (`cargo test -p ap-run`, integrate tree at d7f1bcc):
`gc_weakref_unittest_battery` and `metaclass_new_arity_battery` killed by the
240s probe fence ("hang or pathological growth"). classes_differential and
method_surface_differential green (29/29, 34/34).

**Root cause.** `builtins_modules.rs` pydict `update` (D5 fast path, landed
ae53839) iterates the SOURCE dict with `foreach()`, which holds the source's
`state.borrow()`. The callback calls `d.set()`, which does
`try_borrow_mut` + `sleep(50us)` retry (02.07 GIL v1, dict.rs:209). When
source == target (aliasing — e.g. `sys.modules[cls.__module__].__dict__
.update(cls.__members__)` at vendored_stdlib/enum.py:921/1758 resolving to
the SAME dict during `import enum`), the borrow is never released:
set() sleeps forever. Repro: `d={"a":1}; d.update(d)` (rc=124 at 8s).

**Blast radius.** Every import that self-updates a dict: `import enum`
(240s+ hang), hence unittest, unittest.mock, inspect, dataclasses, and both
differential batteries. Was masked in gate history: main-tree `import enum`
"works" claim predates the D5 fast path (faf7a87-era sets had no gate on this
alias pattern; the 2026-11-05 AGENTS.md "enum imports work" was measured with
the pre-D5 items() clone path).

**Fix.** Identity guard before the foreach:
`if std::ptr::eq(Rc::as_ptr(&o), Rc::as_ptr(&d))` -> items() snapshot path
(1 extra pair-vec per self-update only). Verified: enum import 240s+ -> 1s;
unittest/inspect/dataclasses/mock import chain -> 1s.

**Notes.**
- Same failure mode lurks for any future callback-under-foreach that mutates
  the iterated dict; `copy` is safe (target is fresh). Prefer auditing any
  new `foreach` caller with side effects (ap-gc gcmod.rs callers are read-only).
- meta-kw probe rc=1 on both engines (expected TypeError path; traceback text
  differs pre-Wasm = Class-X, masked? NO — differential compares full stdout;
  gate-5 result will tell whether meta-kw byte-divergence breaks the battery;
  if so it is a known Class-X, not the deadlock).

## Follow-on found by the same main-tree gate run: collections.deque
maxlen semantics broken (3 bugs, 1 test surfaced them; deque_differential
FAILED at stdlib_modules_differential.rs:64, 47/48 passed):

1. Overflow DROP SIDE inverted for append/extend. deque_trim_maxlen always
   removed the LAST item; CPython right-push overflow drops the OLDEST
   (left). appendleft (drops right) was the case that was right.
2. Constructor never trimmed: deque([1,2,3], maxlen=2) returned all 3
   items; CPython keeps the LAST maxlen ([2, 3]).
3. deque.maxlen attribute missing: AttributeError 'object' object has no
   attribute 'maxlen'. Added to native_attr deque arm (int bound or None).

Pre-existing, NOT introduced by the pydict fix. The deque_trim_maxlen helper
+ live-backing-list rework landed today in 0858b53 (perf(ast,fence): deque
O(n^2) ops) — the drop-side and constructor-trim bugs came with it;
deque.maxlen was never surfaced. The deque_differential test predates it
(last touched 0676439, 2026-11-06), so the post-0858b53 gate either didn't
run the ap-cli stdlib differential lane or ran it against a stale binary.
Same binary code present in wave3 integrate (d7f1bcc), which gates only
ap-run suites. Repro ground-truthed against CPython 3.14.7.
