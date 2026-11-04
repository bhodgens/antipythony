# T1: importer resolves imports against the LIVE sys.path list

Status: core fix landed and verified. scrapy import gets past
`ModuleNotFoundError: No module named 'scrapy'` and now dies on a
**different** error (`No module named 'lxml'` — the C-extension dep is not
in the fetched site-packages). Not committed (per task instruction).

## Root cause

The importer kept TWO path stores that had drifted apart:

1. `SYS_PATH: RefCell<Vec<String>>` — the bootstrap snapshot written once by
   `bootstrap_sys_path()` ([script_dir, vendored_stdlib, site-packages,
   ANTIPTYHONY_PATH, extras]). Every importer lookup read THIS.
2. The `PyList` object exposed as `sys.path` — rebuilt as a **fresh list
   snapshot on every `attach_sys_extras()`** call (`import sys`,
   `from sys import ...`).

User-code mutations (`sys.path.insert(0, ...)`, `.append(...)`) mutated the
Python-visible list (2) and were invisible to the finder reading (1).
Probe: `import sys; sys.path.insert(0, '<cache>/site-packages'); import
scrapy` → `ModuleNotFoundError: No module named 'scrapy'` even though
`scrapy/__init__.py` exists, because `find_on_path` iterated the stale Vec.

## What changed (all in crates/ap-run/src/importer.rs)

- New `SYS_PATH_VIEW: RefCell<Option<Gc<PyList>>>` thread-local holding the
  exact PyList object exposed as `sys.path`.
- `sys_path_value()` — creates the list ONCE (seeded from `SYS_PATH`),
  caches it in the view, and returns the SAME object on every later call.
  `attach_sys_extras()` no longer installs a fresh snapshot per call.
- `sys_path_entries()` — the single read path used by the importer: when a
  view exists it reads str entries out of the LIVE PyList (so runtime
  insert/append/delete are visible); otherwise falls back to `SYS_PATH`
  (pre-sys bootstrap reads).
- `find_on_path()` and `augment_with_vendored()` — switched from reading
  `SYS_PATH` directly to `sys_path_entries()` (these two direct reads were
  the actual invisibility bug; the accessors alone changed nothing until
  the finders went through them).
- `bootstrap_sys_path()` — after rewriting `SYS_PATH` it syncs the fresh
  bootstrap content into an existing view (same-process re-bootstrap), so
  the view stays the single source of truth.
- `reset_import_state()` — also drops the view; the next
  `sys_path_value()` rebuilds it, mirroring the fresh-process semantics the
  test suite relies on.

## Probe results

| Probe | Before | After |
|---|---|---|
| `sys.path.insert(0, site-packages); import scrapy` | ModuleNotFoundError: scrapy | gets PAST scrapy; fails on `No module named 'lxml'` (expected: C-ext not fetched) |
| `sys.path.insert(0, dir); import six` (site-packages six.py) | ModuleNotFoundError | SIX OK |
| live-inserted plain module (`zzmod.py` in fresh tmpdir) | only worked via script_dir | ZZMOD OK |
| live-inserted package (`zzpkg/` with `__init__.py`) | — | ZZPKG OK |
| `importlib.resources.files('importlib')` with inserted path | — | callable; returns an object, then `AttributeError: 'object' object has no attribute '__spec__'` at `type(f).__name__` (follow-on gap: the files() anchor lacks `__spec__`; pre-existing, not touched here) |

## Gates

- rust workspace: `cargo test --workspace` — 362 passed / 0 failed /
  1 ignored (53 test binaries, all `ok`).
- `cargo fmt --check` — clean (rc 0).
- corpus (ap-conform, tier0):
  - values: 46/47 (held)
  - fstrings: 34/35 (held)
  - stdlib: 19/48 (held, >= 19)
  - threading: 17/18 (held, >= 17)
- import_differential (6/6) + import_guard_differential (9/9) green.

## Remaining red / next blockers for the scrapy chain

1. **lxml** — `import scrapy` → `scrapy/__init__` chain reaches
   `ModuleNotFoundError: No module named 'lxml'`. lxml is a C-extension
   wheel; the uv `--target` fetch path only handles pure-Python wheels
   (Plan 10 C-API territory), so it cannot come from the current cache.
2. **importlib.resources.files anchor** — returns an anchor object without
   `__spec__`; scrapy.utils.misc and friends will hit this right after the
   lxml wall comes down.
3. Pre-existing single failures in values (1) and fstrings (1) corpora —
   unchanged by this work, not investigated.
