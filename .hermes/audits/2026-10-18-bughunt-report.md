# Bughunt wave 2 - 2026-10-18 (commits 5e8a9f3..b2d4e47 + sibling stack e4864de..27e3894)

Scope: 02.03b enum/warnings, metaclass arity fix, 02.08-full p1 method
surfaces, 02.04 GC salvage commits, plus uncommitted ap-gc WIP (read-only).
Method: parent-direct audits + probes (subagent dispatches all died on
provider timeouts; scopes executed inline instead). Fixes landed on the
detached worktree /tmp/ap-bughunt2 at main.

## FIXED (worktree commits)

| Commit | Finding | Verification |
|--------|---------|--------------|
| 63c460c | `import unittest` died at unittest/signals.py:41 (`weakref.WeakKeyDictionary` missing): (a) `import weakref` resolved to the native stub only - the vendored weakref.py never loaded (augment list missed it); (b) with the vendored file active, `WeakKeyDictionary` lookups still failed because each `ref(k)` made a distinct token and Obj equality/hash were identity-only. Fixed: augment list + `weakref`, and token equality/hash follow `<target_addr>` (CPython weakrefobject.c semantics). | metaclass_arity_differential gate-unittest now PASSES (was the cargo red); `import unittest; print("ok")` green |
| 1861f48 | `tests/README.md` never existed though cited by t07/t16 + traceability messages. Authored from actual enforcement (corpus layout, Cites rule, S-01-38 hash lint, C-33 masking, S-06-15 env pinning, version-checked reference, stale-xfail policy). | content cross-checked against the enforcing test files |
| 6e43b1c | Stale-xfail count is now 126 (was 91). Markers are SHARED decorators spanning suites owned by leaves 01.02/01.03/01.04/02.05 - blanket flipping would churn active leaf surfaces (C-33 corpus monotonicity). Full ledger written: plans/antipythany/02-native-backend/STALE-XFAIL-LEDGER.md (t08 74, t04 29, t02 21, t03 1, t15 1). | pytest -rX census; per-file counts in the ledger |

## ESCALATED (real defects, not fixed - owned elsewhere)

| Sev | Finding | Evidence | Owner |
|-----|---------|----------|-------|
| MED | With-suppression desyncs traceback line tracking: after a suppressed `with` exception, the NEXT traceback reports the wrong line (p5.py: `1 + "s"` at line 9 reported as line 5 `with CM():`). Frame truncate works; the coarse stmt_ordinal line table desyncs on the suppressed unwind. Same root as the documented line-numbers defect (interp.rs:420) - sharper repro for it. | probe /tmp/p5.py: impl vs CPython diff | line-numbers leaf (IR spans) |
| MED | Stored weakrefs can report their referent dead early (`stored()` -> None while referent in scope) - `weakref_alive`/entry lifecycle in ap-gc is the sibling's ACTIVE 02.04 WIP. WeakKeyDictionary survival across gc.collect() not guaranteed until that leaf lands. | probes /tmp/wr20-wr23 | 02.04 GC leaf |
| LOW | `WeakKeyDictionary.__new__` arity: `weakref.WeakKeyDictionary.__new__(cls)` -> `TypeError: __new__() missing 1 required positional argument: 'bases'` - type.__new__ arity path. | probe /tmp/wr24.py | sibling 02.03b/02.04 |
| LOW | Vendored-stdlib purity: 12 files byte-identical to CPython 3.14.7; enum.py has a 1-line deviation (getattr fallback, :496); _collections_abc (80), _py_warnings (3), inspect (16) line diffs; annotationlib.py is an honest self-declared shim (t-strings out of v1 grammar); _contextvars.py + collections.py have no CPython counterpart (runtime-binding shims). LICENSES/README claim holds (docstrings preserved); consider documenting the deviations there. | diff sweep vs installed 3.14.7 | vendored-stdlib owner |

## CLEAN (probed byte-exact vs CPython 3.14.7)

- Enum surface: Color.RED.name/.value/repr/list(Color)/Color(2)/Color["GREEN"] - MATCH.
- % formatting: %d/%s/%%/width/minus/zero-pad/.3f/%x/%X/%o/%r/%c/%e/%g - MATCH.
  (%(mapping)s dict-format is NOT supported - ValueError - tracked gap, see below.)
- issubclass/isinstance with metaclass __instancecheck__ - MATCH.
- set routing: add/discard/in/union/sorted - MATCH.
- GC survival: enum members, class objects, "interned" strings, module globals across 500-alloc pressure - MATCH. rc-cycle with gc.collect() + gc.garbage - MATCH. Deep recursion (900) + gc interplay - MATCH.
- Absorbed-commit check: e4864de carried my tb_frames fix in interp.rs (9 sites) plus its own enum work in frame.rs (class_ns_inst) - nothing half-integrated; the frame.rs change is complete enum machinery, not sweep debris.

## NEW GAP FOUND (not a regression; document in contract gap ledger)

- `"%(k)s=%(v)d" % mapping` (dict right operand) raises
  `ValueError: unsupported format character '('` - % formatting handles
  tuple args only. CPython supports mapping expansion.

## Gates (worktree @ 6e43b1c)

- cargo test --workspace: CARGO_EXIT=0 (metaclass gate now green).
- pytest tests/: 1272 passed, 0 failed, 751 xfailed, 126 xpassed.
- notices --check: OK (151 deps).
