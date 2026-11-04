# Plan 05 (ecosystem T2) - PROGRESS

## Leaf A-1 (RustPython donor adaptations, wave 1) - DISPATCHED 2026-10-24
- deled: impl sa-0-2b6f0c5e + separate-authorship test track sa-1-09518180
  (tests/test_a1_rustpython_backfill.py, xfail-first).
- Baseline: b5ca2fd (wip snapshot of prior in-flight sibling work; full diff
  also at /tmp/antipythony-wip-snapshot-20261024.diff).
- Scope: 6 items per leaves/a-1-rustpython-adaptations-w1.md (errno table,
  binascii qp/uu/hqx, bytes hex/fromhex sep, stat S_IMODE/S_IFMT,
  os.pipe/pipe2 + kill/killpg arms, os.utime ns/dir_fd).
- Gates on completion: cargo workspace 0F, pytest 1912+, corpus
  non-decreasing, xtask notices --check exit 0, differential cases in
  stdlib_modules_differential.rs.
- Orchestrator review protocol on delivery: fresh 5-10 probes vs python3.14
  chosen from the inventory's user-visible contract (not the agent's own
  probes), then commit by explicit pathspec.

## Wave 2 authored - A-2 (2026-10-24)
- leaves/a-2-rustpython-adaptations-w2.md: hashlib sha3/blake2/shake
  (algorithm layer only, no PyHasher), zlib one-shot via flate2, os
  dup/dup2/fstat/dir_fd, os.uname structseq, realpath symlinks, select
  error types. Excludes all needs-redesign items per donor policy.
- Test-mining track dispatched (sa-0-d1405236): mines donor Lib/test/ into
  tests/test_a12_mined_corpus.py, pinned against live python3.14.
- A-2 impl BLOCKED ON A-1 landing (same-file ownership:
  builtins_modules.rs).

## Test-mining track LANDED (sa-0-d1405236, commit 4440dcb)
- tests/test_a12_mined_corpus.py: 32 cases, 15 pass / 17 xfail on the
  pre-A-1 binary (verified by orchestrator run).
- 13 findings; orchestrator VERIFIED firsthand: (F1) silent hard-exit
  rc=0 truncated-stdout on unimplemented call_os arms (os.open, os.symlink
  probed) instead of AttributeError; (F2) native binascii.Error from
  unhexlify escapes try/except (rc=1, catch-block skipped) while manual
  raises are catchable; (F3) crc32(data, init) ignores init (1356327615
  vs CPython 3575297757); (F4) bytearray.fromhex returns bytes-typed
  object (b'\\xab' bytes vs bytearray(b'\\xab')); (F5) stat.filemode
  '?rwxr-xr-x' vs '?rwsr-xr-x' - setuid/setgid/sticky bits dropped.
- Other reported findings NOT yet firsthand-verified: b16decode casefold
  divergence, bytes.hex non-ASCII sep error text, os.kill EPERM ->
  PermissionError mapping + OSError str format, normpath('//a/b'),
  realpath(strict=True) ignored, utime 3-tuple accepted, hashlib object
  model gaps, errno count 56 vs 107.
- Donor-lag findings (not bugs in our tree): donor test_base64 kwargs
  (padded=/ignorechars=) do not exist in 3.14.7; donor test_binascii hqx
  functions (a2b_hqx/b2a_hqx/rle*) were REMOVED in CPython 3.14 - only
  crc_hqx remains. LEAF A-1 ITEM 2 SCOPE NOTE: qp/uu/crc_hqx still valid;
  hqx rle/a2b/b2a functions are 3.14-removed - implement only crc_hqx,
  not the removed hqx family (pin against live 3.14, not the donor file).
- Findings F1/F2 route to an interp-owned fix leaf (builtins_modules.rs
  region is A-1-owned until it lands); the byte-surface findings
  (F3/F4/F5) overlap A-1 items and ride into the A-1 review.

## A-1 REVIEW COMPLETE - LANDED (commit cfe2465, reviewed 2026-10-25)
- NOTE: the commit was made by the repo's control agent, not this session;
  contents match the reviewed state. Orchestrator verification FIRSTHAND:
  cargo workspace 392 passed / 0 failed (incl. gc_weakref_unittest_battery
  now GREEN via the interp.rs type-vs-anything __eq__ identity fix);
  pytest 1951 passed / 6 skipped / 660 xfailed / 82 xpassed exit 0;
  notices --check exit 0; errno count 107 == CPython 107.
- 10 fresh probes vs python3.14: bytes.hex sep forms EXACT, fromhex error
  position EXACT, errno alias/reverse-dict EXACT, stat S_IMODE/S_IFMT/UF_
  EXACT, os.pipe roundtrip OK, binascii qp roundtrip + crc_hqx EXACT.
- 3 RESIDUALS verified, routed into A-2 brief: bytearray.fromhex returns
  bytes-typed object (F4 unresolved); os.utime ns= loses precision
  (...123 reads back ...256 - f64 rounding in utimensat arg path);
  os.kill(1,0) str(e) '(1, Operation not permitted)' vs '[Errno 1] ...'
  (exc_hier py_str, finding only - interp-owned).
- Also still open: stat.filemode setuid/sticky ('?rwxr-xr-x' vs
  '?rwsr-xr-x'); F1 silent hard-exit on unimplemented call_os arms
  (os.open/os.symlink probed) - both routed to the A-2 agent's finding
  queue / a later surface leaf.
- WAVE 2 DISPATCHED (sa-0-4098dc0b): hashlib sha3/blake2/shake, zlib
  one-shot via flate2, os dup/dup2/fstat/dir_fd, os.uname, realpath
  symlinks + '//' preservation, select error types, plus the residuals.

## A-2 REVIEW COMPLETE - LANDED (commit 6fb081b, reviewed 2026-10-25)
- Orchestrator verification FIRSTHAND: cargo workspace 401 passed /
  0 failed (wave-1 was 392; +9 from new differentials); pytest
  1951 passed / 653 xfailed / 89 xpassed / 0 failed; notices --check
  exit 0 (178 deps). Committed by explicit pathspec (5 files, 2680
  insertions). Attribution markers verified (7x "adapted from
  RustPython" in builtins_modules.rs).
- Fresh probes vs python3.14, ALL EXACT: sha3_256(b"abc") digest,
  shake_128 variable-length hexdigest, algorithms_guaranteed,
  zlib roundtrip + adler32/crc32 incl. incremental seed,
  zlib.error text, realpath dangling-symlink + strict FileNotFoundError
  + '//tmp//a2r' collapse, os.dup write/read through the dup'd fd,
  os.fstat st_mode, os.uname fields, utime ns= EXACT (...123 readback).
- Residuals re-checked: kill(1,0) now PermissionError errno 1 (str
  format still pending, A-3); bytearray.fromhex still bytes-typed
  (needs ap-obj bytearray representation - A-3 gap, NOT assigned);
  F2 except-Exception escape CONFIRMED still open for zlib.error
  AND binascii.Error (A-3 item 2).
- A-2 agent-reported gaps routed: blake2 keyed params
  NotImplementedError (small follow-up leaf candidate); kqueue object
  excluded per policy (selectors falls back to PollSelector - behavior
  equivalent); platform.uname processor '' vs 'arm' (documented);
  zlib compressed bytes differ (valid-stream variance, roundtrip pinned).
- stdlib-provenance --check baseline FAIL (121 pre-existing problems)
  NOT touched by this campaign - includes stale select.py/zlib.py
  DEVIATIONS now shadowed by native modules; vendored-tree cleanup is a
  separate queue item.
- A-3 DISPATCHED (sa-0-bdc24419): 5 items - arm-miss hard-exit,
  except-Exception MRO walk, OSError str format, utime arity,
  Opaque type naming + stat_result sequence protocol.
- Remaining open items after A-3: bytearray ap-obj representation
  (interp-owned, needs leaf brief); blake2 keyed params; vendored
  select/zlib DEVIATIONS cleanup.

## A-4 REVIEW (2026-10-25) - ALL 6 ITEMS VERIFIED; COMMIT PENDING
- Orchestrator probes (byte-exact vs python3.14): bytearray repr/type/
  mutation/equality + UNHASHABLE (brief premise corrected: 3.14 bytearray
  does not hash - agent implemented/pinned unhashable, correct); blake2b
  keyed digest + keyed/salt/person + 64-byte key ValueError EXACT;
  filemode full table (0o4755->?rwsr-xr-x, 0o2755->?rwxr-sr-x,
  0o1755->?rwxr-xr-t, T/S variants) EXACT; uname u[0]/u[4]/u[-1]/u[:2]
  tuple EXACT; a2b_base64 strict_mode error text EXACT (brief's
  validate=/altchars= premise also corrected by probe: 3.14 has
  strict_mode=/newline=); notices 190 deps exit 0; provenance
  'OK (454 files, 88 deviated declared)' exit 0; pytest 1951/0.
- COMMIT BLOCKED by sibling churn: the 02.09-pre agent (deleg_632d6db6)
  holds in-flight generators.rs + interp.rs yield-from/si_value work;
  the workspace gate shows 2 transient fails in name_resolution
  (gate_gen_list, bug_c_yield_from_generator) that track the sibling's
  live edits (my isolated re-check showed 6 fails -> 2 fails -> the
  binary flipping between builds). A-4's owned files are clean of it.
- Separation performed: stash-isolated interp/frame/lib, moved the
  sibling's untracked generators.rs to /tmp, restored everything
  (stash pop verified, generators.rs returned, lib.rs mod line
  re-added, build 0 errors, yield-from works in the current tree).
- PATH: commit A-4's owned files by explicit pathspec ONLY (never
  interp.rs/generators.rs/frame.rs/lib.rs which the sibling owns);
  the 2 failing tests are sibling-owned regressions to fix at ITS
  commit gate, not A-4's.
- Continuation agent (sa-0-c87f7258) finished all 5 items + AT_FDCWD
  re-derivation; the control agent committed the tree (1161466, mixed
  with its T2-a pytest work - A-3 hunks identified inside: exc_hier
  OSError str form, interp exc_matches MRO walk, Opaque naming/sequence).
- Orchestrator verification FIRSTHAND (final-state, post-commit):
  stdlib_modules_differential 40/0 (reran after the agent's mid-session
  flaky runs of a3_symlink_readlink - green in final state);
  cargo workspace 407/0; pytest 1951/0; notices --check exit 0.
- Fresh probes: except Exception catches zlib.error AND binascii.Error
  (byte-exact); PermissionError str '[Errno 1] Operation not permitted'
  EXACT; FileNotFoundError carries ': <path>' suffix EXACT; symlink->
  readlink->realpath roundtrip EXACT (islink now real fstatat check);
  type(os.uname()) == 'uname_result', len == 5 EXACT; os.stat st[0] ==
  st.st_mode and len >= 10 EXACT; utime 3-tuple TypeError EXACT.
- ONE RESIDUAL found (routed to A-4): os.uname() SUBSCRIPTING fails -
  u[0] raises TypeError 'object' object is not subscriptable while
  CPython structseq supports it (len works, subscript does not; the
  agent wired subscript_get for Opaque slice_parts but uname_result
  hits a different dispatch path).

## Previously-unverified mining findings - NOW VERIFIED (2026-10-25)
- b16decode MISSING entirely: AttributeError on both impl and CPython
  3.14.7 ('module binascii has no attribute b16decode' - mining report
  wrong that CPython has it; the casefold divergence is moot at 3.14).
  CORRECTION 2026-10-25 (later probe): b64decode/b16decode/b16encode are
  ALL absent from CPython 3.14 - the aliases were removed in 3.14. Our
  impl lacking them is CORRECT, not a gap. Only a2b_base64/b2a_base64
  remain as a surface (A-4 item 2 rewritten accordingly).
- os.kill(1,0): impl raises OSError(1) where CPython raises
  PermissionError(1) - subclass taxonomy missing in call_os error
  mapping. Route with the exc_hier py_str finding.
- realpath(strict=True) silently IGNORED on impl (returns the path where
  CPython 3.14 raises FileNotFoundError errno 2) - already covered by
  A-2 item 5.
- os.utime(p, (1,2,3)) ACCEPTED by impl where CPython raises TypeError
  ("'times' must be either a tuple of two ints or None") - wave-1 agent
  fixed positional times but not tuple-length validation. Routed to A-2
  residuals.

## Leaf T2-a (pytest-minimal) — DISPATCH RESULTS (2026-10-25, uncommitted)

State achieved: `import pytest` (all 48 `_pytest` submodules) and
`pytest --version` -> `pytest 9.1.1` WORK under the runtime. `pytest.main` on a
trivial suite reaches hook execution and dies at the first wrapper hook
(pluggy PEP-342 send semantics) — issue #7 (semantic, gate-blocking).

Blockers cleared this dispatch (all general, no pytest special-casing):
1. classes.rs: Module arm in attr_del_protocol (`del mod.attr` unbinds from
   module_ns) — pygments lexers/formatters module swap.
2. tuple/list-derived instances ARE tuples/lists: alloc backings
   (`__ap_tuple__`/`__ap_list__`), len/iter/contains/subscript/`+` re-wrapped
   in the subclass — pygments `_TokenType(tuple)`, traceback StackSummary.
3. str/dict subclass construction (builtin-base fast path arms) — pygments
   `class include(str)`.
4. tuple/list `__new__` markers (`tuple.__new__(cls, args)` in derived
   __new__) — pygments `class combined(tuple)`.
5. `{*set_display, ...}` starred expansion in set displays — pytest config
   builtin_plugins.
6. typing._GenericAlias CALL constructs the origin class — pytest StashKey.
7. _GenericAlias bases resolve to origin before the metaclass — pytest
   `class CaptureResult(NamedTuple, Generic[AnyStr])` / NamedTupleMeta.
8. `__init_subclass__`/`__subclasshook__` readable+callable on every class —
   NamedTupleMeta post-Generic hook.
9. `__debug__` builtin; sys.monitoring surface (events + inert tool fns) —
   vendored bdb.py (new file, from CPython 3.14).
10. tempfile.TemporaryFile/SpooledTemporaryFile (real file behind Opaque
    TextIOWrapper) — pytest capture.
11. native struct_time subscriptable (`_time.localtime(u)[:6]`) — _pydatetime.
12. functools.update_wrapper real implementation — pytest fixtures.
13. importlib.machinery.PathFinder.find_spec decline marker — pytest
    pathlib.import_path meta_path walk.
14. object.__getattribute__(obj, "__dict__") returns the REAL dict (module_ns
    for modules, attrs otherwise) — pluggy inspect.getattr_static.
15. importer: super_stack snapshot/restore around module imports — nested
    `from X import Y` inside `__init__` before zero-arg super() no longer
    empties the staged context (PytestPluginManager).
16. generator introspection surface (gi_code/gi_frame/gi_running/...).
17. os/shutil terminal_size is a NamedTuple payload (unpackable) — terminal
    width; native NamedTuple <values> iterable.
18. function `__code__.co_flags` carries CO_GENERATOR from def.is_generator —
    pluggy wrapper-hook validation (isgeneratorfunction).
19. T2-fix: exc_matches falls back to name-based check when a Type-kind
    target has no UserClass payload (json.JSONDecodeError catch) — repairs a
    sibling in-flight hunk that SystemError'd the catch site.

Class-X divergences filed: #8 (module delattr note), #9 (str-subclass repr).
Semantic blocker filed: #7 (eagerly-materialized generators vs pluggy
wrapper hooks) — this is the ONLY remaining gate blocker for G-T2a.

Gates verified this dispatch: cargo build clean; `cargo test --workspace`
401 passed / 0 failed; CPython `pytest tests/` 1951 passed / 6 skipped /
652 xfailed / 90 xpassed (>=1912 OK); corpus 270 .py cases (non-decreasing).
NOTE: tree is shared with the A-2 impl sibling (zlib/select/hashlib waves
in flight in builtins_modules.rs) — coordinate before committing.


## Update 2026-10-25 (post PEP-342)

PEP-342 lazy generators LANDED (commit 7c0a645): real send/throw/close/
StopIteration(value) via corosensei stackful fibers. `import pytest` green
(48 _pytest submodules + pluggy). pytest.main blocked at plugin-spec
resolution ([] plugins) + int(ExitCode) - follow-up dispatched.
s018 FLIPPED to passing. s019 ExitStack byte-exact.
Issue #7 (PEP-342) CLOSED by this. Issues #8/#9 remain (byte-divergence).

## T2-gate closure attempt (sa-0-46c08b82, 2026-10-25) - PROGRESS, NOT CLOSED
- pytest.main now: resolves plugins, parses config, FD capture dup2 chains
  work, hook multicall runs, enters wrap_session. Dies in
  Config._ensure_unconfigure -> ExitStack.close teardown with IndexError
  (unisolated; interp frame attribution scrambled hinders tracing).
- Agent fixes (in worktree, reviewed): tempfile TemporaryFile/Spooled
  now carry REAL fds (<fd> via libc::open) + fileno/flush/truncate io
  arms + buffer/raw attr aliasing + TextIOWrapper fd carry - pytest
  FDCapture/snap chains work. dataclasses exec-GLOBALS fix (generated
  fns' freevars threaded per-class, NOT closures) fixes NameError
  __dataclass_dflt_astcache__ for ANY default_factory dataclass -
  VERIFIED firsthand (dataclass OK 42).
- Gates: cargo -p ap-run -p ap-cli 236/0 (verified firsthand).
- FINDINGS routed (interp.rs read-only): (1) isinstance(x, A|B) TypeError
  - is_union gate matches '__union__' but unions are Namespace '_Union'
  (one-line fix at interp.rs:7571; verified repro); (2) interp traceback
  frame attribution shows driver file/line for library frames; (3) _Union
  lacks __args__/__origin__; (4) teardown IndexError unisolated.
- REMAINING for T2 gate: fix the 4 findings (needs interp.rs owners),
  then byte-compare trivial-file terminal summary vs CPython pytest.

## BeautifulRust skills installed (2026-10-25)
- skillspector --no-llm gate run pre-install: rust-review clean;
  rust-best-practices 1 HIGH analysis-evasion flag on one reference file,
  manually inspected = plain prose digest (scanner coverage limit), noted.
- Installed: ~/.hermes/skills/rust-best-practices/ (44 components),
  ~/.hermes/skills/rust-review/. lints/ clippy config not a skill, skipped.


## Update 2026-10-25 (T2 close-out attempt, dispatch 2)

pytest.main PROGRESS this dispatch (uncommitted; sibling files in tree):
- tempfile.TemporaryFile/SpooledTemporaryFile now open a REAL fd (libc::open)
  stored as <fd>; io dispatch gained fileno/flush/truncate; TextIOWrapper
  wrap + io.open carry <fd> through; streams gained .buffer/.raw alias arms
  (builtins_modules.rs). pytest FDCapture start/suspend/resume/dup2 chains work.
- vendored dataclasses: generated __create_fn__ free variables now thread via
  per-class exec GLOBALS instead of closure params — exec-generated nested
  functions were losing/reordering closure cells (freevar/cell mismatch), so
  any dataclass with default_factory fields (e.g. _pytest._code.code
  ExceptionInfoFormatter) raised NameError __dataclass_dflt_astcache__ at
  construction. All dataclasses in-repo construct again.
- builtins.rs: isinstance(x, A | B) accepts _Union namespaces (was TypeError;
  blocked every _pytest traceback format — _pytest._code.Source.__init__ does
  isinstance(obj, tuple | list)).
- pytest.main now: plugin-spec resolves, Config.parse succeeds, capture
  starts/stops, hooks run, session runs; still fails at Config._ensure_unconfigure
  ExitStack.close teardown with IndexError (frame attribution scrambled; root
  cause NOT yet isolated — needs interp trace).
- REMAINING READ-ONLY-FILE FINDINGS for the next dispatch:
  1. interp.rs:7571 is_union gate matches Opaque "__union__" but unions are
     Namespace "_Union" (interp.rs:3013) — isinstance(x, A|B) with a builtin
     subject still TypeErrors in the interp path even though builtins.rs
     handles it. One-line: add Namespace/"_Union" to that is_union arm (and
     classes.rs:2066 gate). Repro: `isinstance("x", tuple | list)` -> TypeError
     (builtins path fixed; interp path still raises).
  2. interp frame attribution garbage in tracebacks (frames of the driver line
     shown for every library frame) — breaks _pytest traceback walking
     (ExceptionInfo.getrepr -> repr_traceback) and debugging.
- Baseline for the byte-compare (CPython 3.14.7, staged pytest 9.1.1, pluggy
  1.7.0, iniconfig 2.3.1, packaging 26.3): "1 failed, 1 passed, 1 skipped",
  exit 1 — captured in this session log.
- Gates this dispatch: cargo test -p ap-run -p ap-cli GREEN (0 failed); full
  cargo test --workspace dispatched.

3. _Union namespaces (interp.rs:3013) lack __args__/__origin__ surfaces
   (repr'd <_Union object>; getattr(__args__) is None) — typing introspection
   of unions will need these.
