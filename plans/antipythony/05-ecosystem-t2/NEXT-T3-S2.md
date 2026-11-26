# NEXT-T3-S2 — Plan 05 leaf: T3 S2 re-entrant lazy-import wall (issue #20)

Date: 2026-11-26. Worktree: `~/.git/worktrees/leaf-t3s2` (branch `tier1-t3s2`,
off main `a38f490`). Target dir: `target-t3s2` (CARGO_BUILD_JOBS=4; one heavy
cargo lane held; logs under `logs-t3s2/`, never /tmp).

## VERDICT: PARTIAL — two interp walls root-caused + FIXED (commit 65051a1);
the S2 pydantic chain advances past functional_validators/json_schema and
stops at a NEW wall (recorded exactly). The recorded S2 mechanism
(re-entrant lazy import through PEP-562 __getattr__) did NOT reproduce as
described — the real mechanism is different (see CORRECTIONS).

## CORRECTIONS to the NEXT-T3.md S2 record (re-verified first, as instructed)

1. The recorded mechanism ("nested from-import of collections.abc → typing →
   pydantic __getattr__ re-entry") was NOT reproducible: four stdlib-only
   minimal repros (pkg __getattr__ lazy-imports submodule; submodule body
   imports stdlib that walks back into the pkg) all PASS byte-identical
   under antipythony (probe sources in logs-t3s2/probe/minrepro*.py). The
   'cannot import name BaseModel' error is a SECONDARY effect: pydantic.main's
   body fails on an unrelated TypeError, exec_compiled drops main from
   sys.modules, and the ImportError surfaces through the __getattr__ call.
   The REAL blockers, in order, with the pure-py pydantic_core shim
   (t3chat/deps_py/, probe-only, real types instead of the probe .so's str
   markers):
   a. Probe .so exposes SchemaValidator as a STR (not a type) →
      `ValSer = TypeVar('ValSer', bound=SchemaValidator | ...)` at
      pydantic/_internal/_mock_val_ser.py:17 raises
      `TypeError: unsupported operand type(s) for |: 'str' and 'type'`.
      Extension-surface gap (NEXT-T4 W6 class), out of interp scope.
   b. FIXED (methods.rs): Opaque `typing._GenericAlias` re-subscription
      (`Callable[[T, X], T][T]` in pydantic.functional_validators stmt 36-42
      TypeAlias assignments) fell into the structseq catch-all arm →
      `TypeError: 'tuple' object is not subscriptable`. Minimal repro:
      logs-t3s2/probe/alias_sub2.py. New arm: positional __ap_typevar__
      substitution (mirrors the Namespace `_GenericAlias` arm).
   c. FIXED (sre.rs + runtime_state.rs + threading.rs): MATCH_TABLE's
      SRE_TABLE_CAP=1024 ledger eviction dropped rows for LIVE Match objects
      (finditer over >1024-line sources; vendored ast.py
      _splitlines_no_ff `match[0]` → `TypeError: bad receiver for
      Match.__getitem__`), blocking pydantic.json_schema. Now: rows are
      liveness-pinned (SRE_MATCH_PINS_T weak pins, c018 SLOT_STORAGE_PINS
      shape); `sre_match_evict_dead()` rides evict_stale_tables; cap
      eviction removed; recycled addresses overwrite.
   d. NEW WALL (OPEN, next leaf): pydantic._internal._model_construction
      stmt 33 — `@dataclass_transform(...)` applied to
      `class ModelMetaclass(ABCMeta)` — fails
      `AttributeError: 'type' object has no attribute '__instancecheck__'`.
      Individual repros of the shape (dct on plain class / ABCMeta class /
      metaclass __instancecheck__ hook isinstance) all PASS; the failure
      needs the real pydantic import context (frame attribution scrambled
      per F-INTERP-2, so the innermost quoted frame "aliases.py 3" is not
      trustworthy). Interp-owned leaf; the failing isinstance consult is
      somewhere in the classes.rs/builtins.rs metaclass-hook arms.

## Chain progress (staged pydantic 2.14.1 + pure-py pydantic_core shim)

Before this leaf: pydantic.main body died at _mock_val_ser.py:17 (str | type).
After: S0/S1 GREEN unchanged; pydantic.functional_validators FULL body GREEN;
pydantic.json_schema GREEN; every pydantic._internal module GREEN except
_model_construction (wall d). `from pydantic import BaseModel` still fails
(via main → _model_construction), with the __instancecheck__ AttributeError
as the root (visible in `import pydantic.main` directly: probe_main_first
traceback, exact bytes in the session).

The REAL pydantic_core probe .so (t3chat/deps/libpydantic_core.so) still
exposes str markers — gate 3 of the leaf contract can only be exercised
through the shim until the S1 extension surface ships real types (Plan 10.04).

## Gates (executed)

- `cargo test -p ap-cli --test stdlib_modules_differential`: 48 passed / 0 failed.
- `cargo test -p ap-run --test re_differential`: 42/0; re_fuzz 4/0;
  import_differential 6/0; classes_differential 29/0;
  method_surface_differential 34/0; gc_differential 2/0.
- `scripts/unsafe_report.sh --diff`: zero new unsafe.
- No stdlib-only regression probe added: both fixes are structural (alias
  placeholder arm + match table liveness) not expressible as a single
  stdlib-only probe without staging a custom expected-bytes shape; the
  probes under logs-t3s2/probe/ carry the minimal repros for the next leaf.

## Machine policy

One cargo lane; CARGO_BUILD_JOBS=4; artifacts under the worktree
(logs-t3s2/, t3chat/deps_py/, t3chat/probe_*.py uncommitted by convention).
Commit by explicit pathspec; FUZZ_SEEDS.txt modification in the worktree is a
SIBLING's side effect — NOT committed (verified unrelated to this leaf).
