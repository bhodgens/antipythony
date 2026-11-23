# NEXT-T3 — Plan 05 leaf: hermes-agent chat-turn reachability (pydantic + httpx + rich)

Date: 2026-11-23. Worktree: `~/.git/worktrees/leaf-t3` (branch `tier1-t3`,
off main `4fa6e56`). Target dir: `target-t3` (CARGO_BUILD_JOBS=4; one heavy
cargo lane held; logs under `~/.git/worktrees/`, never /tmp).

## VERDICT: REACH (honest stage list) — S0/S1 GREEN, S2 blocked at the
pydantic BaseModel dataclass chain, S3/S4/S5 blocked by vendored_stdlib
gaps (recorded with exact bytes). Four wrapper-class fixes landed; three
gates re-verified green.

## Stage list (driver `t3chat/t3chat.py`, run via
`APX_EXT_DIR=t3chat/deps antipythony run t3chat/t3chat.py`)

- **S0 GREEN — `import pydantic_core`**: the rebuild-simulated probe
  extension (`tests/ce-s1/pydantic_core.c` → `build-ext` →
  `t3chat/deps/libpydantic_core.so`, loaded via the `APX_EXT_DIR` seam)
  imports and inits through the built binary. RC=0.
- **S1 GREEN — `import pydantic` (2.14.1)**: the pure-py face imports and
  reports VERSION after the wrapper-class fixes below. pydantic's own
  wheel set is refused by `antipythony add` (APX-CEXT: pydantic_core ships
  a native component → total refusal per C-07/C-11), so the pure-py wheel
  was staged manually into `$ANTIPTYHONY_CACHE/site-packages` (finding
  F-STAGE below).
- **S2 BLOCKED — `class ChatMessage(BaseModel)`**: `from pydantic import
  BaseModel` triggers pydantic's PEP-562 `__getattr__` →
  `import_module('.main', 'pydantic')` → pydantic.main → aliases.py module
  body `from collections.abc import Callable` → typing chain → re-entrant
  pydantic `__getattr__` while pydantic is partially initialized →
  `ImportError: cannot import name 'BaseModel' from 'pydantic'`. The
  traceback frames are attribution-scrambled (known interp frame bug); the
  reproducible inner failure chain is:
  dataclasses._add_slots → inspect.unwrap(member) →
  functools update_wrapper frames → `_update_func_cell_for__class__`:
  `idx = f.__code__.co_freevars.index("__class__")` on members whose
  `__code__`/`__wrapped__` reads resolve to synthetic markers. Structural:
  re-entrant lazy imports of a partially-initialized package (CPython
  allows it because the from-import consults the module `__getattr__`
  AFTER sys.modules lookup and our nested from-import of `collections.abc`
  → typing → pydantic `__getattr__` re-enters). NOT fixed — recorded.
- **S3 BLOCKED — `import httpx`** (0.28.1 staged via `antipythony add`,
  green):
  ```
  ImportError: cannot import name 'parse_http_list' from 'urllib.request'
    (/Users/caimlas/git/antipythony/vendored_stdlib/urllib/request.py)
  ```
  httpx/_auth.py:9 does `from urllib.request import parse_http_list`; the
  vendored trimmed urllib/request.py (6 defs total) lacks it. VENDORED GAP
  (do-not-edit per sibling policy; report-only). Fix = restore
  `parse_http_list` (+ `_parse_proxy`-adjacent helpers it needs) to the
  vendored request.py in a vendored-owned leaf.
- **S5 BLOCKED — `from rich.panel import Panel`** (rich 15.0.0 staged,
  green): first wall `ModuleNotFoundError: No module named 'colorsys'`
  (132-entry vendored_stdlib lacks the pure-py colorsys; CPython 3.14.7's
  colorsys.py staged beside the driver made rich proceed past it), then:
  ```
  File ".../vendored_stdlib/typing.py", line 3077, in NamedTupleMeta.__new__
      set_name = type(val).__set_name__
  io.UnsupportedOperation: fileno
  ```
  rich's console init measures the terminal through sys.stdout.fileno();
  our stdout wrapper raises `io.UnsupportedOperation` where CPython's
  shutil.get_terminal_size chain catches OSError/ValueError (the raise
  escapes inside NamedTupleMeta during the typing chain). INTERP SURFACE
  gap (stdout fileno semantics under `antipythony run`), report-only.

## Wrapper-class fixes LANDED (all pinned by the failing imports, each
general — no pydantic/httpx special-casing)

1. **module `__getattr__` self-deadlock (interp.rs Module attr arm)** —
   the PEP-562 fallback held the module_ns guard ACROSS the
   `__getattr__` call; typing.__getattr__ runs `bool | None` →
   eval_binop make_union → sys_modules_get("typing") → module_ns.lock()
   = same-thread non-reentrant Mutex deadlock. Sampled: PC parked in
   make_union +1144, RSS flat. Fix: snapshot (attr, __getattr__, __name__)
   under the lock, drop it, THEN call. (typing.ContextManager /
   AsyncContextManager hung forever before; instant now.)
2. **`__spec__` seeded into module globals pre-exec (importer.rs
   load_into)** — pydantic/__init__ reads `__spec__.parent` in a
   module-level dict literal (PEP 562 dynamic-import table); CPython sets
   module.__spec__ BEFORE the body runs, we didn't (NameError
   `__spec__` at pydantic/__init__ line ~252). Minimal
   name/parent/origin spec namespace seeded; parent = the package's own
   name for packages (CPython ModuleSpec.parent semantics — pydantic is a
   package, `__spec__.parent` must be `pydantic`, not `''`), dotted
   prefix otherwise.
3. **`importlib.import_module` native seam: 2-arg form + relative names
   (builtins_modules.rs call_importlib)** — the seam treated a 2-arg call
   as (self, name) and imported the PACKAGE as the module
   (import_module('.warnings','pydantic') returned `pydantic`); and
   relative names were imported raw. Fixed: args[0] is the name
   (CPython shape), `package` accepted positionally or by keyword, and
   leading-dot names resolve against package exactly like vendored
   _bootstrap._resolve_name (level counting + rsplit + beyond-top-level
   ImportError).
4. **object-virtual marker isinstance/`__code__` shape (builtins.rs
   type_matches + interp.rs BuiltinFn attr arm)** —
   `isinstance(marker_fn, types.FunctionType)` answered True
   (type_name "function" catch-all) so dataclasses._add_slots treated
   synthetic `__object_virtual_*` members as raw functions; and marker
   `__code__` reads raised AttributeError inside
   `_update_func_cell_for__class__` (uncatchable there → ImportError).
   Fixed: markers no longer match FunctionType; `__object_virtual_*` /
   `__ap_*` BuiltinFns answer `__code__` with an empty-freevars stub
   (co_freevars.index raises ValueError → dataclasses' caught no-op path,
   CPython-equivalent outcome).

## Probe extension (tests/ce-s1/pydantic_core.c) additions — my file

- module attrs at init: `__version__` = "2.50.1" (pydantic.version
  compares against its pinned compatible core version — must match, else
  version.py raises SystemError), `PydanticUndefined`, `core_schema`,
  plus the full measured pydantic import surface as S1 markers
  (SchemaValidator, SchemaSerializer, CoreConfig, CoreSchema, MISSING,
  PydanticCustomError, PydanticKnownError, PydanticOmit,
  PydanticUndefinedType, Some, to_jsonable_python, Url, MultiHostUrl,
  build_profile). Reason: pydantic's `__getattr__` dynamic-import loop
  getattr()s EVERY same-module name from pydantic_core after the first
  hit — any missing name AttributeError-breaks the whole lazy surface.
- Probe gate re-verified RC=0 (driver output byte-stable for the tagged
  lines: MODEL type / GATE 42 / NEG_RAISED / TYPE_RAISED / EXC
  ValidationError / probe surface lines).

## Findings (report-only, owned by other surfaces)

- F-STAGE (ap-cli add): pydantic is unusable via `antipythony add`
  because the APX-CEXT purity refusal rejects the whole resolved set
  (pydantic + pydantic-core in one resolve). A pure-py-only mode
  (`add pydantic --pure` staging pydantic + pydantic-core's PURE sibling
  deps and leaving the native part to the ce path) would unblock T3/T4
  consumers without weakening C-07. Exact bytes in the session log:
  `APX-CEXT: pydantic_core-2.50.5.dist-info ships a native component
  (... WHEEL (non-pure wheel tag cp314-cp314-macosx_11_0_arm64)) ...
  antipythony v1 refuses it.`
- F-VENDORED-1: vendored_stdlib/urllib/request.py lacks
  `parse_http_list` (httpx/_auth.py:9 needs it). The vendored file is a
  trimmed T1-scrapy copy (6 defs).
- F-VENDORED-2: vendored_stdlib lacks `colorsys` (rich needs it; staged
  CPython 3.14.7's colorsys.py beside the driver as a workaround in this
  leaf, file left in place for the next leaf to vendor properly).
- F-INTERP-1: stdout fileno semantics — rich's terminal measure chain
  hits `io.UnsupportedOperation: fileno` escaping through
  typing.NamedTupleMeta (vendored typing.py:3077); CPython's
  shutil.get_terminal_size catches this class of error. Under
  `antipythony run`, stdout is not a tty — the fileno probe should raise
  an OSError subclass that the stdlib's existing catches absorb.
- F-INTERP-2 (known, confirmed again): traceback frame attribution
  scrambled for library frames — every wall this leaf hit showed
  functools/enum/dataclasses frames for what were actually
  pydantic/typing frames; root-causing each wall cost extra rounds.
- F-STRUCT-1 (S2, structural): re-entrant lazy import of a
  partially-initialized package (pydantic.main → aliases → collections.abc
  → typing → pydantic.__getattr__ re-entry). CPython tolerates this
  because from-import consults module `__getattr__` on the
  sys.modules-resolved module; our nested path re-enters the importer.
  Needs an interp-owned leaf; exact repro in this file's S2 section.

## Gates (executed, not claimed)

- `cargo test -p ap-ce` (target-t3): **36 passed / 0 failed** (unchanged).
- `cargo test -p ap-cli --test stdlib_modules_differential`: **48 passed /
  0 failed** (unchanged).
- pydantic_core probe driver through the binary: **RC=0**, tagged lines
  byte-stable vs the NEXT-PYDANTIC-PROBE record.
- vendored_stdlib: ZERO edits (sibling safety; colorsys staged at project
  level only, reported as F-VENDORED-2).

## Machine policy

One cargo lane; CARGO_BUILD_JOBS=4; artifacts/logs under
~/.git/worktrees/leaf-t3*. Commits by explicit pathspec; no `git add -A`.
