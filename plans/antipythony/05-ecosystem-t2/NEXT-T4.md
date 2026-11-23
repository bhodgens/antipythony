# NEXT-T4 — Plan 05 T4 reachability leaf session record

Date: 2026-11-23. Worktree: `~/.git/worktrees/leaf-t4` (branch `tier1-t4`, off
main `4fa6e56`). Target dir: `target-t4` (CARGO_BUILD_JOBS=4; one heavy cargo
lane held).

## VERDICT: reachability stage map landed + 4 engine fixes

`import fastapi` (0.165.0) via `antipythony run` reaches **typing_extensions
full import + pydantic_core dlopen-import GREEN**; the chain blocks at
`pydantic/version.py:7` — `from pydantic_core import __version__` — because the
only registered extension is the S1 probe `.so`, whose namespace is
probe-only (`__name__/__doc__/probe_surface/raise_validation/ValidationError/
Model`). That is an S2 extension-completeness wall (Plan 10.04), not an engine
wall. CPython arm (`/opt/homebrew/bin/python3.14`, same site-packages) imports
fastapi fully and drives the GET route; our arm reproduces stages 1-2 of the
import chain identically and stops at the pydantic.version wall.

## Stage map (captured)

| Stage | CPython | antipythony (this leaf) |
|---|---|---|
| `import fastapi` begins | OK | OK |
| anyio import (incl. `_lazyimport` AST lazy-map) | OK | OK (after W2/W3/W5) |
| typing_extensions full import | OK | OK (after W4) |
| pydantic_core import (dlopen S1 probe .so) | OK | OK (with APX_EXT_DIR=ext) |
| pydantic import | OK 2.14.4 | WALL W6 (probe .so lacks `__version__`) |
| fastapi defined + GET route driven | OK `{'msg': 'pong', 'n': 1}` | not reached |

Driver: `scratch/t4/t4_probe.py`; staged chain: `scratch/t4/s62.py`.
Run env: `ANTIPTYHONY_PATH=<site-packages> APX_EXT_DIR=<ext> APX_GC_THRESHOLD=100000000`.

## Walls (exact)

- **W1 `type.mro()` missing** — `vendored_stdlib/email/_policybase.py:113`
  `_extend_docstrings` walks `base.mro()`; our `type` object had no `mro`
  attr → `AttributeError: 'type' object has no attribute 'mro'`. FIXED.
- **W2 PEP 634 class patterns unimplemented** — `anyio/_lazyimport.py:127`
  `case ast.If():` → `SystemError: class patterns need 02.03 classes`
  (interp.rs:11814 stub). IMPLEMENTED (isinstance + __match_args__ positional
  + kw-attr subpatterns; non-class subject = no-match, per CPython).
- **W3 unbound str classification surface** — `annotated_types` line 359
  `Predicate(str.islower)` → `AttributeError: 'type' object has no attribute
  'islower'`. FIXED via the builtin_method_exists name-gate (semantics already
  routed through the pystr native path).
- **W4 module `__getattr__` re-entry self-deadlock** — `typing.ContextManager`
  PEP 562 hook builds `bool | None` (defaults tuple) whose `make_union` re-locks
  the SAME module's `module_ns` mutex; interp.rs:15813 held the guard across the
  Python call. Worse: the first fix `drop(ns)` dropped the SHADOWED `&PyDict`
  binding — a no-op — while the MutexGuard stayed held (the deadlock survived a
  build!). FIXED by restructuring the Module arm: clone the needed Values, drop
  `ns_guard` by name before `call_value_pub`, re-lock only for `__spec__`.
- **W5 GC auto-collect re-entrancy during import (OPEN, ap-gc)** —
  `anyio.abc._streams` import → `attach_module_spec` → `PyObject::new` →
  `gc_register` → auto-collect → `edges_of` blocks on a `PyList::items` mutex
  held elsewhere on the same thread (captured via `sample(1)`;
  `/tmp/s80.sample` in-session). Workaround used for reachability:
  `APX_GC_THRESHOLD=100000000`. NOT FIXED here (ap-gc owner; separate leaf).
  Symptom class matches the 02.09b "std Mutex is not reentrant" note.
- **W6 pydantic_core API surface (S2, out of leaf scope)** — the S1 probe .so
  exposes only the probe symbols; real pydantic needs `__version__`,
  `SchemaValidator`, `ArgsKwargs`, ... (docs/ce-scan-symbols-full.json's
  measured 169 covers the C-side demand; the PYTHON-visible name surface is the
  next work queue for 10.04).

## Fixes landed (engine, additive)

- `classes.rs` `class_attr_get`: `"mro"` arm → parked-owner BuiltinFn
  `type.mro` marker (next to `__subclasses__`).
- `interp.rs` call handler: `type.mro` arm → rebuild the payload's LIVE MRO as
  a List (`class_value_for_public` per entry; CPython: mro() returns list,
  `__mro__` stays tuple).
- `interp.rs` `pattern_match`: `Pattern::Class` implemented (was a SystemError
  stub). isinstance gate via `check_isinstance_public`; positional arms read
  `__match_args__` order then attribute sub-match; kwargs sub-match by attr
  equality. Pinned CPython-identical on If/non-If shapes.
- `builtins.rs`: `check_isinstance_public` re-expose.
- `interp.rs` Module arm: restructured so the `module_ns` MutexGuard is dropped
  by NAME before invoking a PEP 562 `__getattr__` (W4; see above).
- `interp.rs` set/frozenset virtual-dunder fallback: EXCLUDED `__wrapped__`
  (CPython set/frozenset do not define it; the marker made `inspect.unwrap`
  descend past frozenset values → `'function' object has no attribute
  '__code__'` in dataclasses `_add_slots`).
- `interp.rs` `builtin_method_exists`: str classification/isX + padding/case
  family (islower/isupper/isalpha/isdigit/isalnum/isspace/istitle/isascii/
  isidentifier/isprintable/isnumeric/isdecimal/lstrip/rstrip/rfind/rindex/
  rjust/ljust/center/zfill/capitalize/title/casefold/swapcase/splitlines/
  removeprefix/removesuffix/expandtabs/find/maketrans) — unbound `str.<m>`
  reads now resolve; semantics already existed on the pystr call path.
- `builtins_modules.rs` `call_importlib` `import_module`: RELATIVE names
  resolve against `package` (CPython `_resolve_name`: level 1 = the package
  arg itself, each extra dot drops one trailing component). Binding-shape
  aware (plain call: args[0]=name; bare-marker bind: args[0]=module object).
  Without this, anyio's lazy `import_module('._streams', 'anyio.abc')` raised
  and the lazy `__getattr__` recursed to RecursionError.

## Gates (executed)

- `cargo test -p ap-cli --test stdlib_modules_differential`: **48 passed /
  0 failed** (unchanged).
- `cargo test -p ap-run`: 208 passed / 0 failed (incl. classes/import/gc
  differentials).
- `cargo test -p ap-ce`: 36 / 0. `cargo test -p ap-obj`: 71 / 0.
- `cargo test -p ap-cli`: 83 passed / 0 failed (ast_getattr_fence needs
  `target{,/release}/ap-run-probe` — copied from target-t4; hybrid gates need
  ap-host + ap-aot-crt{-hybrid} built — done).
- pytest slice: `tests/t04_fastapi_reach` **7 passed** (NEW, additive);
  t05/t06/t11/t14 + a1/a12: 718 passed with 8 failures that reproduce on the
  STASHED BASE tree (V048 expected-files missing ×3, e016-e020 header-cites
  ×5 — pre-existing on main @4fa6e56; not from this leaf). t14 CLI gates pass
  once the binary/staticlibs are visible at `target/{debug,release}` (leaf
  uses CARGO_TARGET_DIR=target-t4; copies made).

## Honest scope notes

- fastapi import itself works through anyio + typing_extensions; the wall is
  the pydantic extension surface (W6). No vendored_stdlib file needed changes —
  all four engine fixes are additive interp/builtins surfaces.
- W5 (GC auto-collect re-entrancy during import) is a real runtime hazard for
  the whole ecosystem-T2/T4 family; filed for an ap-gc owner leaf with the
  captured stack. Workaround env is documented above.
- The scratch/ dir (site-packages staging, drivers, probe outputs) is NOT
  committed (uv fetch products; reproucible via the documented uv command).

## Machine policy

One cargo lane; CARGO_BUILD_JOBS=4; worktree+artifacts under
~/.git/worktrees/leaf-t4. Explicit pathspecs throughout.
