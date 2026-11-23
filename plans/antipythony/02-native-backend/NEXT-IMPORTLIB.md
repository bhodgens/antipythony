# NEXT-IMPORTLIB — session record (leaf tier1-importlib, 2026-11-23)

Branch: `tier1-importlib` (worktree `~/.git/worktrees/leaf-importlib`,
base main @ 3bafad9). CARGO_TARGET_DIR=target-importlib,
CARGO_BUILD_JOBS=4, one heavy cargo lane.

## What landed

Commit a54a996 — the three T5 wall-2 surfaces from
`tests/t5-probe/importlib_machinery_stub_repro.py`, all in owned files
(crates/ap-run/src/{builtins.rs, builtins_modules.rs, importer.rs}):

1. **find_spec directory-package miss (FIXED — highest value)**:
   `call_importlib_util("find_spec")` previously answered None for any
   module outside vendored_stdlib/native registry, even with site-packages
   first on sys.path. Now: imported modules return
   `sys.modules[name].__spec__`; otherwise a LIVE sys.path scan per entry
   order — `<dir>/<name>/__init__.py` (package spec:
   submodule_search_locations + parent=name) wins over the sibling
   `<dir>/<name>.py`; dotted names auto-import the parent first (CPython
   contract). `find_spec('torch')` now byte-matches CPython 3.14.7
   (origin / submodule_search_locations / parent / cached / repr); miss
   still returns None (s047 pin intact).

2. **SourceFileLoader exec_module END-TO-END**: the machinery loader
   classes are real constructible class values
   (`__module__ == "_frozen_importlib_external"`, CPython-shaped class
   dict). `spec_from_file_location` builds a full ModuleSpec
   (name/origin/loader/has_location/cached(__pycache__/<stem>.cpython-314.pyc)/
   parent/submodule_search_locations + CPython __repr__);
   `module_from_spec` seeds a real module ns
   (__name__/__doc__/__package__/__file__/__path__/__spec__/__loader__,
   __package__ = own name for packages, dotted parent otherwise).
   `spec.loader.exec_module(mod)` runs the real .py through the importer
   (verified: typing_extensions.py executes, its names resolve).

3. **machinery stub ctor NameError (FIXED)**:
   `im.SourceFileLoader("m", path)` constructs. Ctor seam: the class
   value carries a `__new__` marker (`__ap_sfl_new__`) so
   `classes::instantiate`'s custom-__new__ path dispatches through
   `builtins::call_builtin`'s `"__ap_"` arm — the only ctor route
   reachable WITHOUT touching interp.rs/classes.rs (both OFF-LIMITS this
   leaf; an initial approach intercepted `__init__` in classes.rs and was
   reverted). Bound methods ride the instance as pre-bound BoundMethods
   over `__ap_sfl__.<m>` markers
   (create_module/exec_module/get_code/get_data/get_filename/get_source/
   is_package/path_stats/source_to_code) handled in
   `call_native_marker_impl` → `call_source_file_loader_method`.

New importer seam: `importer::exec_module_file(interp, module, name,
path)` — compile + run a real .py INTO an existing module object
(load_into ns sharing; pre/post share mirrors exec_compiled; sys.modules
NOT auto-touched, matching direct exec_module calls).

## Gates (all executed on this tree)

1. Repro probe `tests/t5-probe/importlib_machinery_stub_repro.py`:
   ctor OK, exec_module OK, `__module__` correct — no NameError, no
   AttributeError.
2. E2E probe (ctor attrs/repr/get_filename/is_package; spec
   has_location/cached/parent/repr; module_from_spec attrs; exec_module
   on typing_extensions.py; find_spec torch/yaml/miss): byte-matches
   python3.14.7 for every fixed surface.
3. ap-conform stdlib slice `tests/corpus/stdlib/s0*.py`: **48/48**
   (identical stdout + exit code; s047 importlib_basics untouched).
4. `cargo test -p ap-run --release`: **18 suites, 208 passed / 0 failed,
   RC=0** (needed debug `ap-run-probe` + `ap-cli` `ap-run-eval`
   pre-built — the documented fresh-worktree harness gotchas).
5. `pytest tests/t05_values tests/t06_classes`: 8 failed / 408 passed —
   failure set diffed IDENTICAL against base main @ 3bafad9 (all
   pre-existing env-shape fails; zero new, zero regressions).

## Structural leftovers (recorded, NOT hacked around)

- `SourceFileLoader.__dict__` lists our marker names; `__init__` appears
  as `__new__` in the class dict (ctor is __new__-routed). CPython lists
  `__init__` + path_stats/_cache_bytecode/set_data. `repr(loader)` and
  `loader.name/.path/.get_filename()` are CPython-exact.
- `get_code`/`source_to_code` return None (v0: no compiled-code
  production; exec path is exec_module).
- `path_stats` returns None (mtime/size unimplemented).
- The native/vendored face of find_spec still returns a name-only spec
  (pre-existing v0 contract) — only the on-disk sys.path faces got full
  specs.
- CPython's spec `cached` is a lazy property; ours is precomputed at
  spec build (same value, no invalidation semantics).
- Sibling side-effects on this worktree (reverted before commit): a
  cargo-fmt sweep had touched embedded_stdlib/data.rs + frame.rs +
  threading* + interp.rs/classes.rs (fmt-only churn, off-limits files) —
  `git checkout --` restored them; FUZZ_SEEDS.txt append (re sibling
  fuzz lane) reverted.
