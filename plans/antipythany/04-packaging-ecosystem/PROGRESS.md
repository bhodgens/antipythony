# Plan 04 progress log

## Leaves 04.01 + 04.02 — uv wheel-fetch packaging + sys.path integration — LANDED (working tree, uncommitted)

### ap-pack crate (new)
- `crates/ap-pack/`: manifest (toml+lock schema, hand-rolled emit/parse for the
  flat T-14 G-1 shape — loud errors on shape violations), uv subprocess wrapper
  (`uv pip install --target <staging> --no-compile`; helpful uv-absent error
  naming the install docs; `ANTPYTHANY_UV` override; version pin constant
  UV_MIN_MAJOR=0 documented in docs/packaging.md), purity filter (file layer:
  `.so/.pyd/.dylib/.dll/.a/.o`; tag layer: any WHEEL Tag not `*-none-any`),
  cache (site-packages + content-addressed build entries keyed SHA-256 over
  input/lockfile/toolchain/backend/opt-level; `ANTPYTHANY_CACHE` override).
- New workspace member; new deps: sha2 (ap-pack), zip (ap-cli dev-dep for the
  fixture wheel). THIRD_PARTY_NOTICES regenerated (173 deps, --check green).

### CLI verbs (ap-cli main.rs rewritten)
- `new <name>` scaffold (manifest + `<name>.py` seed printing
  "hello, antipythany" — the T-14 bytes), `add <pkg|file:path|path>`,
  `run [--no-cache] <file.py>`, `build [-o out]`, `cache gc`. `--tier`
  forwarded (tier-0 only, loud reject otherwise). `--wasm` rejected loudly
  (04.04 territory).

### Add-time purity gate
- Two-layer inspection over a STAGING install (cache-root staging dir, same
  volume); refusal promotes nothing (C-11 total refusal: no cache write, no
  lock entry). Diagnostic carries APX-CEXT + package + full native module path
  + "C extension" + both policy pointers + `file:1:1` (C-08/C-10).

### sys.path integration (04.02 run)
- `importer::bootstrap_sys_path` order: script_dir, vendored_stdlib, FETCHED
  site-packages (only when the project manifest walking up from script dir
  records deps AND the dir exists), ANTPYTHANY_PATH, extras. Both ap-cli `run`
  and ap-run-eval get it (interp.rs script_dir ""→"." fix for bare filenames).

### Runtime fixes en route (cross-file, flagged)
1. sys.version_info >= (3,3): NamedTuple vs tuple ordering — six.py does this
   at module scope (interp.rs eval_compare NamedTuple arm).
2. `importlib.util.spec_from_loader` added to the native surface (six binds it
   at module scope).
3. ModuleType-derived classes (types.ModuleType = type(sys)): ctor consumes
   (name, doc=None) and allocates a REAL module object (ObjKind::Module +
   module_ns) — six.moves is a _LazyModule(types.ModuleType) instance.
4. importer submodule ADOPTION: dotted-walk miss falls back to a parent
   module-ns attribute that is a module (or module-derived) object —
   `import six.moves` resolves the _LazyModule registered on six (six's
   meta-path importer is out of v0 scope). Plus import_from_module falls back
   to the full attr protocol (class descriptors on module-backed objects —
   MovedAttribute resolution for six.moves.zip).
5. operator.methodcaller is now the real FACTORY (`methodcaller(name, ...)` →
   callable; extras serialized into the `<methodcaller:...>` marker name,
   mirroring `<partial:...>`). s029's `methodcaller("shout", 3)(Thing())`
   shape preserved.
6. struct shim (new structmod.rs): fixed-width scalars b/B/h/H/i/I/l/L/q/Q/?/x,
   big/little-endian, Struct objects with pack/unpack/calcsize/size/format;
   six needs Struct(">B").pack. Beyond the served formats: fail-loud
   NotImplementedError (S-05 struct OUT-of-v1 preserved for the rest).
7. `exec` builtin: statement execution over the current globals
   (parse→lower→exec_block; FnDef registration like exec_module). six execs
   `def raise_from(...)` source at import. Explicit globals dict: fail-loud.
8. latin-1 codec (str.encode / bytes.decode): byte-identical U+0000..FF map
   (six.b uses s.encode("latin-1")); encode error text byte-pinned.
9. __init_subclass__ delivery fixed: (a) plain `def __init_subclass__(cls,...)`
   now binds cls (implicit classmethod); (b) hook search spans ALL bases' MROs
   (typing's _SpecialGenericAlias(_NotIterable, _BaseGenericAlias, _root=True)
   carries the hook on _Final via the SECOND base); (c) type.__new__ forwards
   leftover class kwargs when a consuming hook exists up the bases' MRO
   (object's default still rejects with the pinned text).

### Gates (all verified firsthand)
- six END-TO-END BYTE-EXACT vs CPython 3.14 (same six 1.17.0 from the same
  wheel): version/PY3/PY2/MAXSIZE/string_types/moves/b/u/iteritems — stdout,
  stderr, exit code all diff-clean (crates/ap-cli/tests/
  packaging_differential.rs::add_six_import_use_byte_exact).
- packaging add/purity/lock: ADD succeeds (pure accept), lock pins
  packaging==26.3 with uv provenance. Runtime run-compare is a DOCUMENTED GAP:
  packaging 26 module bodies (version.py `class _BaseVersion(NamedTuple)`,
  markers/requirements/specifiers/utils) import vendored-typing
  NamedTuple/lru_cache machinery our typing stub does not yet serve
  (AttributeError 'function' object has no attribute 'cache_clear' — pre-existing
  Plan-02 typing-stub surface, exposed not caused by this leaf). Interactive
  verification at leaf time: Version compare/str, Marker evaluate, Requirement
  parse, SpecifierSet construction all byte-exact before the deeper typing
  paths land; the gap is tracked in docs/packaging.md.
- lxml add → APX-CEXT diagnostic with all C-08/C-10 fields; nothing enters
  cache/lock (C-11).
- uv absent → nonzero exit, error names uv + install docs.
- Cache: cold run populates ~/.cache/antipythany/build/<hash>; warm run adds
  zero entries; --no-cache bypasses; cache gc clears; post-gc run works.
- sys.path ORDERING PROOF (inside a run with six added):
      1. <project dir>
      2. <repo>/vendored_stdlib
      3. ~/.cache/antipythany/site-packages
- T-14 suite: 8 passed, 12 xfailed, 2 xpassed (new/add verbs XPASS their
  xfail markers — strict=False, informational). NEW differential suite
  crates/ap-cli/tests/packaging_differential.rs: 7 passed.
- rust workspace: 362 passed / 0 failed (xtask notices regenerated for the
  new deps; --check green). fmt clean. pytest tests/ minus threading:
  1815 passed / 0 failed; t10_threading: 97 passed / 0 failed.
- Corpus at this worktree (ap-conform vs python3.14, unchanged from the HEAD
  baseline measured with my changes stashed): stdlib 19/48, values 46/47,
  classes 27/33, exceptions 13/15, gc 16/17, threading 17/18. NOTE: the
  20/48 stdlib figure in PROGRESS-02 was measured with sibling work in flight;
  HEAD-without-my-changes also measures 19/48 on this machine today.

### Honest gaps (documented in docs/packaging.md)
- packaging 26 runtime run-compare blocked on vendored-typing NamedTuple/
  lru_cache internals (pre-existing Plan-02 typing-stub surface).
- exec with explicit globals dict: fail-loud NotImplementedError.
- `build` artifact is a deterministic launcher + content-addressed staging
  entry; the true static artifact is 02.06 AoT. Byte-reproducibility across
  clean caches holds by construction.
- struct shim serves fixed-width scalars only; the rest of struct fails loud.
- NOT COMMITTED per protocol.
