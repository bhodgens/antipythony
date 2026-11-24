# NEXT-LINT — build-ext linkage lint row (leaf, 2026-11-23)

Task: gate the leaf-1005a framework-linkage SEGV gotcha (NEXT-TOPLEVEL-COMPLETION
item 0) — a build_ext_gate row asserting built .so files carry ZERO
Python.framework entries + docs warning about the manual-cc SEGV failure mode.

Worktree: `~/.git/worktrees/leaf-lint` (branch `tier1-lint`, off main
`1722168`). Target dir: `target-lint`. CARGO_BUILD_JOBS=4; one heavy cargo
lane at a time.

## What landed

1. `crates/ap-cli/tests/build_ext_gate.rs` — new row
   `build_ext_output_has_zero_framework_load_commands`: builds a trivial
   `linklint.c` extension through `antipythony build-ext`, runs
   `otool -L` on the output, asserts ZERO `Python.framework` load-command
   lines. Skips cleanly ("skipped: otool unavailable" / macOS-only) on
   non-macOS hosts so CI stays green on linux. Failure message names the
   manual-cc SEGV failure mode and points at docs/build-ext.md.
2. `docs/build-ext.md` — new section "NEVER link the Python framework (the
   manual-cc SEGV failure mode)": the never-do warning (-lpython /
   -framework Python / python3.X-config --ldflags), the root-caused
   intermittent dyld binding SEGV description (PyModule_Create2 sometimes
   bound to the framework copy → wrong runtime → crash), the one-line
   verification (`otool -L libhello.so | grep -c Python.framework` must
   print 0), rebuild-through-the-verb remediation, and the gate name.
   Gate section updated to list the new row.
3. EN-ROUTE FIX (harness, not the lint itself):
   `crates/ap-cli/tests/ast_getattr_fence.rs` — the fence resolved its
   probe binary via hardcoded `CARGO_MANIFEST_DIR/../../target/{debug,
   release}/ap-run-probe`, IGNORING CARGO_TARGET_DIR. Under this task's
   binding `CARGO_TARGET_DIR=$PWD/target-lint` policy the release candidate
   fell through to the MAIN CHECKOUT's stale release binary (baked paths,
   2026-11-22 build, predates e7e5021 importlib machinery) and the debug
   candidate didn't exist → spawn NotFound (or worse: silent use of the
   stale main-checkout binary — exactly the AGENTS.md stale-binary gotcha).
   Fixed to honor CARGO_TARGET_DIR (fall back to <workspace>/target when
   unset — unchanged behavior for default builds). With a fresh release
   `ap-run-probe` in target-lint, the fence runs the 1000-iteration probe
   in ~210s inside its 600s deadline: suite 2/2.

## Gates (executed, not claimed)

- `cargo test -p ap-cli --test build_ext_gate`: **5 passed / 0 failed**
  (4 pre-existing rows + the new linkage lint row).
- `cargo test -p ap-cli`: ALL suites green EXCEPT one PRE-EXISTING
  failure (below); everything else 0 failed including hybrid_gate 7/7,
  packaging_differential 6/7, ast_getattr_fence 2/2.
- `cargo test -p ap-cli --test stdlib_modules_differential`: **48 passed /
  0 failed** (the task's stdlib diff 48/48 gate).
- Machine policy held: one cargo lane at a time, CARGO_BUILD_JOBS=4,
  worktree+logs under ~/.git/worktrees/.

## PRE-EXISTING FAILURE FILED (NOT from this leaf; off-limits files)

`packaging_differential::add_six_import_use_byte_exact` FAILS at trunk
base `1722168` with a FRESH build: `import six` under antipythony raises
`AttributeError: 'object' object has no attribute
'submodule_search_locations'` at six.py:987
(`__spec__.submodule_search_locations = []`), CPython rc=0 / ours rc=1.
Root cause chain: the importlib machinery band (e7e5021, tier1-importlib
merge 2026-11-23) — module `__spec__` objects expose
`submodule_search_locations` as a non-assignable attribute of the shared
'object' (module_from_spec modules even report `m.__spec__ is spec` False
in both stale and fresh builds; fresh builds at least construct the spec).
The main checkout's target/debug binaries are STALE (mtime 2026-11-22
23:45, predate e7e5021) which is why trunk "looked" green: `import six`
works under the stale binary and fails under any fresh one. Filed for the
importlib-band owner; NOT fixed here (crates/ap-run off-limits to this
leaf). Any fresh full rebuild on trunk will trip this — do not
misattribute to new leaves.

## Sibling safety

- Explicit pathspecs only; no `git add -A`.
- No crates/ap-run source touched; owned files only + the one harness
  CARGO_TARGET_DIR fix (ast_getattr_fence.rs), which is required for ANY
  leaf working under the target-lint convention to run ap-cli gates.
- Stray probe processes killed after each diagnostic run; no background
  daemons left.
