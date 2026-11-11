# ANTIPYTHONY - Project Context (auto-loaded by Hermes from this repo)

## What this project is
Python 3.14-compatible runtime and compiler, written in Rust. Compiles Python to
(1) single static native binaries and (2) WasmGC modules. Tracing GC (MMTk), no
GIL, uv-backed deps at build time. Byte-exact differential conformance vs CPython
3.14.0 is the exit gate for every phase. Working name: antipythony (repo dir:
~/git/anticoagulant).

## Where the truth lives
- Plan tree: `plans/antipythony/` - master.md is the contract; one orchestrator per
  plan (00 foundation, 01 frontend/IR, 02 native, 03 wasm, 04 packaging,
  10 C-extensions hail mary); tests/README.md is the separate-authorship test track.
- Locked decisions are in master.md. Do not re-litigate them in leaves.
- This file: operational state + durable conventions only.

## License state
- Project: Apache-2.0 WITH LLVM-exception.
- pon (can1357/Can Bölük, me@can.ac): DUAL LICENSE MIT/Apache-2.0 GRANTED via
  email, 2026-10; confirmed by user - full code donor, adapt/vendor freely with
  attribution. BEFORE first pon-derived commit: (1) LICENSES/ entries (standard
  MIT + Apache texts), (2) attribution headers on adapted files. Levers
  01.03 (pon-ir lowering study) and 02.05 (pon tier-1 inline-cache/OSR/deopt
  study) - DONE: both study docs landed (docs/pon-study-lowering.md +
  docs/pon-study-tier1.md @ 1237c75, clone d0a1717, ongoing updates
  7238cc5). LICENSES/ entries + attribution headers still required
  before first pon-DERIVED CODE commit (studies cite, don't vendor).
- Nuitka: AGPL - subprocess tool only, never vendor/link.
- p2w: design reference only (wrong language).

## Execution protocol (binding)
- Leaves fit one 256K-context agent. Test leaves are written from the spec pack
  (docs/spec/), by different agents, in a separate pass - never from impl code.
- Corpus only grows. Never weaken a test to pass a gate.
- Orchestrator (main model) reviews in-session; parents determine completion.
- "Commit then implement" / "fix all" = pre-authorized per user convention.
- Specs before code: nothing dispatches from Plans 01+ until 00.05 spec pack lands.
- Repo gates run in CI, NOT git hooks. This machine's global
  `core.hooksPath=~/.githooks` (LFS hooks only - verified: `git config
  --get core.hooksPath`) silently disables repo-local hooks, so any future
  repo hook is a no-op here. If hooks are ever wanted, onboarding must set
  `git config core.hooksPath .githooks` per-repo. See docs/development.md.
- Machine load policy (binding for local gate runs): ONE heavy cargo test
  lane at a time on this host; parallel cargo builds during suites use
  CARGO_BUILD_JOBS=4. Cause: 2026-11-07 WindowServer watchdog stall from a
  1009-process parallel-cargo flood (power cycle required). Full policy:
  docs/test-interpreter-lock-policy.md.

## Current status (update this block when it changes)
- Plans 00+01 COMPLETE. Plan 02 nearly complete: 02.01 values, 02.02 executor,
  02.03/02.03b classes+metaclass, 02.04 GC (weakref surfaces, unittest gate,
  MMTk design doc docs/gc-mmtk-swap.md), 02.08-lite + 02.08-full p1 (25+
  native modules, method surfaces) ALL COMMITTED.
- unittest/pathlib/enum/weakref IMPORTS ALL WORK (vendored stdlib chain).
- BUGHUNT 2026-10-17: 7 fixes landed; report in .hermes/audits/.
- PLAN 02 REMAINING: 02.05 tier-1 remainder (02.05c typed regions + 02.05d
  deopt + 02.05e driver + 02.05f OSR + 02.05h bench) + 02.06 AoT + 02.08
  method surfaces + 02.09 no-GIL swap (T121 >=3x GREEN gate) + 02.10
  Go-scheduler (stretch).
- 02.05 SPLIT COMPLETE 2026-11-11 (all merged to main; exit gate PASSED):
  02.05a DONE; t1_016 FIXED - root cause was NOT frame/stack sizing: clif.rs
  emit_cfg emitted Inst::LoadGlobal's dst as the RAW TempId instead of
  temp_slot() (86d0c09), so ANY JIT-routed call site crashed TypeError
  'NoneType' not callable; min repro depth-independent `def k(n): return
  len(n)`; ignored differential test re-enabled. 02.05b DONE (a4cfea3):
  TypeObject type_version + surgery_version AtomicU64 counters (ap-obj
  versioning.rs builtins_version for row 18), bump sites bound in
  ap-run/classes.rs, ALL ic-version-audit rows COVERED/documented-N/A;
  IC recording lands in FeedbackCells. 02.05g DONE (d42212d):
  ap-gc/rootscan.rs SafepointMap registry + fp-chain walker (fp-chain
  SHIPPED, ~2-4% FP cost, not force-enabled until pointer-typed CLIF values
  exist - 02.05c resolved: i64 repr has none, conservative floor is the
  root story, documented-N/A). 02.05c DONE (104f76c/a912979/e25fa20):
  typed regions i64-smi repr, VALUE-level guards (deliberate divergence
  from pon's counter-guarded temps - recorded in PROGRESS-0205c.md),
  region-entry==block0 soundness rule, cold-twin CONTINUATION at exit block
  (not restart-from-entry), h_call ABI arity bug fixed (call bodies had
  silently tree-walked since 02.05a). 02.05d DONE (7fb7337/154e51c):
  deopt.rs epoch/thrash backoff (threshold 64, shift cap 6, DISABLED pin
  after 8 epochs) + en-route fix: SSA temp map cleared per emit (stale-temp
  wrong-value hazard). 02.05e DONE (bfa9f9c/66fcf77): tierup.rs background
  TierUpDriver (CAS install, snapshot-at-enqueue, pin-until-retire,
  APX_SYNC_TIERUP=1 debug mode); UNION FIXES (orchestrator-hand-merged
  after leaf + fixer both failed): (1) DEOPT_TABLE was thread_local! -
  compiler thread's Tier1 store invisible to mutator, notes no-oped, pin
  never fired -> now process-global OnceLock<Mutex>; (2) unconditional
  per-call CALLFAST-DBG eprintln filled the stderr pipe and deadlocked
  race-stress at its own 300s deadline -> env-gated (APX_CALLDBG) + race
  harness drains pipes concurrently. 02.05f DONE (9d0a9f4): boxed OSR
  end-to-end - back-edge poll (no-op in plain baseline), OsrTransferBuffer
  canonical layout, OSR-BY-CALL entry ABI, routed through the background
  driver, [OSR-STATS] under APX_OSR_STATS; typed-OSR stays out (study
  problem #2); For-loop OSR + generator OSR + no-GIL registry filed as
  follow-ups. 02.05h DONE (1732b51): BENCH GATE PASS - 2.07x conservative
  geomean (min/max estimator, 3 runs), 2.20x median/median, >=1.5x floor
  cleared; per-probe int-sum 3.20x, int-mul 2.65x, str-len 1.68x, str-concat
  1.24x; slice is FUNCTION-shaped (module-level loops never reach
  jit_call_body - the old bench corpus measured startup noise at 0.96x);
  docs/tier1-bench.md + tests/bench/tier1/. Merged-trunk gates after f+h:
  356 passed / 0 failed / 1 ignored (35 suites); tier_mode_differential 3/3;
  race-stress 100/100; osr_gate 2/2; orchestrator re-measured geomean 2.14x
  byte-identical. FILED (post-split): 02.05f emit bug - builtin
  Call(local-Name arg) feeding BinOp in while loop fails Cranelift verify
  (invalid block ref block3), transparent tier-0 fallback, byte-identical
  (caps tier-1 coverage for that shape); adaptive-mode traceback linenos
  imprecise (JIT frames attributed to def line).
- 02.06 AoT BINARY COMPLETE 2026-11-11 (merged to main; macOS aarch64
  PRIMARY VERIFIED): `antipythony build app.py -o app` -> native Mach-O
  (17.7MB, links only libSystem). Commits 7c34de0e..87e2cd0f (branch was
  history-sanitized: a `git add -A` had swept 29,200 target-0206/ build
  artifacts into 4 commits - filter-branch stripped them; /target-* now
  gitignored). Architecture: aot.rs cranelift-object emit (shared cfg→CLIF
  lowering, static feedback snapshots), ap-aot-crt staticlib crate IS the
  entry (rss-guard, manifest registration, catch_unwind, 256MB-stack
  interpreter thread, exit-code map - replaces spec's aot_entry.rs shape,
  documented), link.rs static assembly, ap-conform --impl-binary mode.
  Gates verified on trunk by orchestrator: hello + classes/inheritance +
  module-For probes byte-identical vs CPython 3.14 (module-For = tree-walk
  fallback per 02.05 discipline, loud [AOT-BUILD] notice); T-09 30-case
  slice through the BINARY 30/30; startup 0.69x of JIT run (soft gate);
  GC-heavy 20M-alloc probe green; catch_unwind rc 134 no raw backtrace;
  notices --check OK (345 deps); workspace 540/0. MUSL: BLOCKED - no musl
  toolchain on host (rustup absent, Homebrew rustc 1.94); --target
  linux-musl implemented + fails loudly naming the missing toolchain;
  15MB musl size gate unmeasurable here, report-only per T-09 GAP-2
  (installing toolchain = user decision). FILED follow-ups: stdlib .py
  not embedded yet (native registry unconditional; embedding = future
  work); Stmt::For at module level AOT-unsupported (tree-walk fallback);
  For-loop OSR + generator OSR + no-GIL OSR registry (from 02.05f).
  PLAN 02 REMAINING: 02.08 method surfaces + 02.09 no-GIL swap (T121 >=3x
  GREEN gate) + 02.10 Go-scheduler (stretch).
- ROBUSTNESS-CI CAMPAIGN: plan tree at `plans/robustness-ci/` (master.md +
  7 branches, 24 leaves) implements ALL findings of
  .hermes/audits/robustness-review-2026-10-19.md (CI core, test integrity,
  lint gates, provenance, fuzzing, CLI/docs, gated decomposition). Status
  2026-11-07: waves A-D largely LANDED (.github/workflows/ci.yml +
  nightly-fuzz.yml + clippy.toml at HEAD; interpreter-gate watchdog
  e9196b8 + its TOCTOU follow-up 46729d4). Branch 06 decomposition
  hard-gated on Plan 02; clippy -D burn-down (~49 errors) and
  stdlib-provenance --check remediation queued.
- Status block verified at 46729d4 2026-11-07 (fresh re-measure on the
  merged trunk): 686 commits; `cargo test --workspace` 61 suites ok /
  444 passed / 0 failed / 1 ignored (log
  ~/.git/worktrees/wave3/main-gate-final.log); `pytest tests/ -q`
  1977 passed / 6 skipped / 656 xfailed / 93 xpassed, exit 0 (measured
  at 47be826; the 93 xpass are the known stale awaits-02.07/02.08 ledger
  markers); `xtask notices --check` OK (336 deps). Corpus inventory:
  274 corpus .py under tests/corpus; pass-rate counts: see
  plans/**/PROGRESS.md, source of truth. Previous block (stale): "94
  commits. rust 289/0/1. Corpus 8 families/270/696".
- T1 SCRAPY: DONE 2026-10-25. scrapy 2.19.0 imports and runs; file:// spider
  items.jsonl byte-identical vs CPython (verified). Gate doc: docs/t1-gate.md.
  Chain: a14fe0d (sys.path), 9fe23b4 (lxml shim + interpreter wave), a021802
  (lxml 24/24), cfe2465 (A-1 RustPython adaptations). ~25 blockers cleared.
  Known residual Class-X divergences tracked as GitHub issues #2-#5
  (byte-divergence label) per Amendment A-3/D6 (86cefd8): pre-Wasm standard is
  works-best-effort; byte-exact returns post-Plan-03. A-1 RustPython wave 1
  landed: errno 108 pairs, binascii qp/uu/hqx, bytes hex sep, os pipe/kill/
  stat/utime arms, ClosureMarker fix, function-not-iterable fix (cfe2465).
  Corpus 149/178=84%+; stdlib 28/48.
- T2 PYTEST (gate GREEN 2026-10-28, waves 6-23 + 25-57 in NEXT-STEPS.md):
  PEP-342 lazy generators (7c0a645) cleared the pluggy-wrapper blocker
  (issue #7 CLOSED 2026-10-31). T2 minimal gate: pytest.main runs a 3-test
  trivial suite to completion, verdict-identical with CPython (1 failed /
  1 passed / 1 skipped, rc=1). Tier-2 fixtures pass: tmp_path, monkeypatch,
  generator fixtures, parametrize. Key fixes: ev.instance preservation
  (5424f1b), per-exception tracebacks with per-frame FILE attribution
  (98b2964), stream-base subclasses (CaptureIO), fd-backed os primitives,
  builtin method-surface type gating (list.keys no longer exists),
  tuple.index start/stop, union __args__/__origin__/isinstance surfaces.
  Remaining Tier-2 polish: capsys fixture teardown int-None (tracked in
  NEXT-STEPS waves 50-57), traceback frame linenos imprecise (degrades).
- T2 PYTEST FULL GATE (GREEN 2026-11-04, W112 campaign ee4239e..a6b84ac):
  DEFAULT-CONFIG gate (cacheprovider ENABLED, no -p no:cacheprovider)
  verdict-identical with CPython: 1 failed / 1 passed / 1 skipped,
  MAIN_RC=1, zero INTERNALERROR (t2g104/t2g105/t2g106). The interim
  -p no:cacheprovider config is RETIRED. Root causes fixed: iter.rs
  swallowed generator-body errors as exhaustion (degenerate tracebacks
  -> _rawentry/ishidden crashes); gen.throw dropped instance __ap_tb__;
  os.umask returned None instead of the previous mask (cacheprovider
  _make_cachedir 0o777 - umask); open('wb') was a text stream;
  shutil.rmtree was a v1 stub. (Was: compile(PyCF_ONLY_AST) a name-surface
  stub - now a REAL tree since the W114 AST campaign; see the T2
  REGRESSION item below.) Known residual: the staged-source degradation
  path may interact with real AST trees (see T2 regression). GOTCHA: the
  rename left STALE test binaries with pre-rename baked paths
  (target/debug/antipythony, harness_selfcheck, xtask_gates) - if
  gates fail with missing vendored_stdlib/NotFound, touch the test
  files and rebuild before debugging.
- T1 SCRAPY CRAWL (in progress; campaign doc NEXT-STEPS-W112.md rounds
  20-26): CRAWL COMPLETES END-TO-END 2026-11-05 (bridge-e6.py: Spider
  opened -> PARSE CALLED -> Spider closed (finished) -> HARNESS-DONE
  RC=0). Root fixes a7ff45b: async-for over user async-iterators
  (Stmt::For async arm + aiter/anext protocol), fiber super-context
  relay (pending_super_ctx rode GenFrame/GenState through resume_live -
  zero-arg super() inside async-gen methods worked again), staged
  scrapy LocalWeakReferencedCache __contains__ shim (membership fell
  to __getitem__ sequence scan -> infinite loop). Bad-yield bridge:
  vendored tasks.py Task.__step recognizes twisted Deferred ->
  asFuture(loop) (leaf-verified: CPython raises the same 'bad yield'
  on raw-Deferred-await; ours now bridges). PREMISS CORRECTION:
  "install_reactor HANGS" (HANDOFF-2026-11-03) was NOT a hang - the
  scrapy+twisted import chain legitimately takes 70-120s in DEBUG
  builds (~270 modules re-parsed per process); probes need timeout
  >=300s. Current blocker (leaf running): response.text crashes in
  w3lib/encoding.py resolve_encoding -> blocks the items.jsonl
  byte-compare gate (docs/t1-gate.md standard).
- T2 REGRESSION CLOSED 2026-11-05 (W117 verify): default-config plain
  driver (/tmp/w112_t2fix/plain_driver.py) on final tree = 1F/1P/1S,
  MAIN_RC=1, zero INTERNALERROR — verdict-identical with CPython 3.14.
  Residual (NOT the gate): the old trap-driver variant
  (/tmp/w110_t2drv_trap.py, pluggy _multicall monkeypatch) hits a
  different INTERNALERROR: fixtures.py _matchfactories
  AttributeError 'type' object has no attribute 'nodeid' on the skip
  path — file as a leaf if it ever blocks wave work.
- OOM CAMPAIGN CLOSED 2026-11-05 (session record; details
  docs/import-leak-recheck-2026-11-05.md + docs/ast-hang-diagnosis-2026-11-05.md):
  the box-OOMing 9-44GB leaks on import-heavy probes were the pre-fix
  vendored _ast.py bulk-getattr loop (native _ast getattr materializes
  ~110 fresh class clones per access, OUTSIDE the GC heap) - owner
  CLOSED by W114 round-38 (363600e); full scrapy I0-I8 chain now
  RC=0/843-879MB. Harness hardened (dea6c03): ap-run differential
  suites run probes as fenced subprocesses (6GB RSS/240s deadline
  kills, cross-process lock). GC auto-collect was O(n^2) (full-heap
  scan every 700 allocs) - fixed with generational cascade +
  growth-triggered major GC (bcc0a04/743e277); 3M-obj probe
  never-finishes -> 100s/60MB. Sibling WIP absorbed at gate-green
  (9befdb4). RESIDUALS (all filed + reconciled 2026-11-05):
  (a) #11 CLOSED-as-misattributed: fenced ground-truth shows current
  native _ast getattr is interned (50x136 loop = 32KB; build__ast
  registers via interned_type_pub) - the leak was the pre-fix shim +
  the (b) recursion per level; regression guard shipped as
  gc_differential::ast_getattr_identity_fence (#14 CLOSED);
  (b) importer.rs:654-678 _ast branch inserts sys.modules AFTER body
  exec (generic path pre-inserts at :1240) - re-entrancy hazard for
  any future self-importing _ast; CLOSED 2026-11-05 by 8bab377
  (W117: pre-insert partial module before exec, drop-on-failed-body;
  the hazard fired in production as the W114 _ast.py self-import
  recursion = the codecs.lookup 8.6GB/150MB-per-level leak and the
  `import ast`+parse hang; both fenced-green after the fix);
  (c) #13 CLOSED 2026-11-07: native multibyte CJK codecs COMPLETE (wave
  a8841dc). ENGINE byte-exact: 1056/1056 encode+decode differential vs
  CPython 3.14 (24 codecs); differential_probe.py 530/530; e2e w3lib
  PROBE-GREEN; s037 corpus gate green. Root fixes this wave: gen_tables.py
  exhaustive enumeration (hand-written ranges had lost euc_jp's JIS X 0208
  block + johab hanja rows); gb18030 4-byte Gb4Run odometric runs table;
  iso2022 per-charset table resolution + P1/P2 split + plane-2 removal from
  _3/_2004 (ref rejects it); codecs.lookup dedupe poisoning FIXED via
  Frame.frame_id 4th key (Box addr recycling collided same-shaped wrapper
  modules); utf_7 native + utf_16/32 primitives; CPython-exact
  UnicodeError message escapes. T1 scrapy CJK response.text blocker
  CLEARED. GOTCHA: cargo does NOT relink after mbc_tables.rs changes —
  `cargo clean -p ap-run` before probing or you test stale tables.
  (d) DONE: RSS-fenced probe = ast_getattr_identity_fence.
- OOM CAMPAIGN #2 CLOSED 2026-11-06 (growth-class sweep; incident + fix
  record docs/oom-bughunt-2026-11-06-{rss-fiber-weakreg,static-growth-class}.md,
  verdicts docs/oom-fix-verification-2026-11-06.md, harnesses
  docs/harness-review-2026-11-06.md): the 11:40 fork-bomb stall was
  ast_getattr_fence re-exec'ing the test binary unargued (fixed
  0676439); harness hardening sweep 9f7e191 (gate+bounded-wait on every
  interpreter-spawning suite, zero unbounded Command waits). 15 growth
  findings fixed in waves: 7eb65a4 F2 weakref prune+O(n) drain,
  9ee848c F1 fiber reservation 64->8MB (AP_GEN_STACK_MB; 2MB SIGBUSes
  pluggy debug chain), 090ad43 G1 iterator registry prune + no container
  clone (1.63GB@1k breaks -> 11.7MB flat), eb6f7b4 G3 lxml DOCS LRU,
  72b6fb5 F3 StringIO in-place extend (485->33MB @200k), eece027 G2 sre
  tables SRE_TABLE_CAP 1024 + wired 512-entry RE_CACHE (re.compile(p)
  is re.compile(p) now holds), c80550c G4 (superseded), c96f41c G8
  exhausted-gen retire, 67d3e09 G5 cell cache + G6 (superseded),
  86c4cdc G7 drain + G10 tb_frames resume-depth + F4 recursionlimit
  backstop (SIGABRT -> RecursionError). ADVERSARIAL VERIFY (76cebda)
  FAILED G4 (dedupe key was per-call frame ptr) + G6 (prefix+exit clone
  2x output resident); REPAIRED 44c016a/5a633ed (code-object-identity
  key w/ closure-bypass gate - shared-entry closure corruption caught
  mid-repair; stdout_capture opt-in, fd-1 bytes sha256-identical);
  304674b docs. Zero regressions: 898 pytest + 165 differential.
  RESIDUALS: closure-carrying nested defs in hot loops ~3MB/1k calls
  (per-object cell storage, filed); M1-M4 threading tables + M11 lxml
  XPATH open; pre-existing pdb NameError in pluggy hook (bisect
  pending, present @4709a20). SIBLING RECOVERY (5c07b9f/59426db/
  16507e0): session 20261106_095004_5de0d7 (bughunt wave, idle) WIP
  landed - classes.rs base_type_ref TypeRef cache KEPT, attr_get_opt
  cache arm DROPPED (poisoned interned types: import inspect +
  SystemExit died in enum.py __set_name__ _flag_mask_ |= object;
  documented in commit). Correctness bugs 1+2 FIXED (c553a64):
  metaclass __init__ now receives class kwargs; super_stack
  __setitem__ dispatch is single-pop. WAVE-3 MEMPERF CAMPAIGN CLOSED
  2026-11-07: the ast_getattr_leak_fence perf regression owner landed
  fence-green at 0858b53 (deque O(n^2) ops + iterator-table leak); the
  fixer stacks merged to trunk via 47be826 (g1 interpreter hot path
  d7f1bcc + g2 parser/IR quadratics + g3 GC borrow scans + g4 threading
  eviction with evict_stale_tables wired into the major-GC collect
  drain, join path untouched). Consolidated audit:
  .hermes/audits/2026-11-07-memperf-consolidated.md (19-item wave plan
  for the remaining OPEN rows incl. FILE_SRC cap (M07/disclosed M10),
  pull_iter borrow hang risk (M03), lru_cache
  M29, path-finder M13). TWO gate-found bugs fixed same day: (i)
  3a80a4b pydict.update self-alias deadlock (foreach borrow + set()
  retry-sleep never released when source==target — hung `import enum`,
  hence unittest/inspect/dataclasses probes at the 240s fence; enum
  import 240s->1s; doc .hermes/audits/
  2026-11-07-dict-self-update-deadlock.md), (ii) 3fcc046 deque maxlen
  semantics (overflow dropped the WRONG end for append/extend — old
  comment had appendleft/append inverted; constructor never trimmed
  deque(iterable, maxlen); deque.maxlen attr missing) - both
  ground-truthed vs CPython 3.14.7, workspace gate green after each.
  Gate-keep: 46729d4 fixed the new gate_wait_expiry_smoke TOCTOU
  (probe-then-reacquire lost a race against a starting suite; now a
  single LOCK_EX|LOCK_NB + clean SKIP). RESIDUALS: closure-carrying
  nested defs ~3MB/1k (filed); M11 lxml XPATH open; pre-existing pdb
  NameError in pluggy hook (bisect pending). NOTE 2026-11-07: the
  sibling repr_fuzz 35-min stall + "shrink loop" was the stdin-EOF
  harness bug (eceb1b7 fix: ref_batch must DROP child.stdin so CPython
  reference reads EOF; suite now ~6.5s).
- T1 CRAWL history (waves 90-110, pre-round-20 state - superseded by
  the block above, kept for fix provenance):
  W110-5 LANDED (85432c9): real coroutine frames - async def bodies
  execute, send() runs to completion with true StopIteration values,
  await binds (PEP-492 drive loop with gen_yield relay), async-gen
  protocol surface, iscoroutine co_flags. En-route fixes:
  time.monotonic/perf_counter were ~always 0.0 (all call_later timers
  frozen), select.poll multiplied ms timeouts by 1000, CPython
  private-name mangling (__x -> _Cls__x aliases), sys.is_finalizing.
  Follow-ups (d7b827d): membership over generators/iterators,
  warnings.py prefers _py_warnings hook protocol (catch_warnings
  records now work). Chain: spider opens, Scheduler/RFPDupeFilter
  construct, DL resumes, exit 0. Findings: docs/wave-110-crawl.md.
  GC issue #10 CLOSED (764ad5b):
  gc013 was stale (passes), gc009 was an unraisable source-echo
  indent bug (trim fix), gc015 was finalize-token alive attr
  shadowing (token exemption at the Opaque early-return); fence
  gc018-020 landed; gc family 20/20. Triage:
  docs/gc-issue10-triage.md.
  Wave-110 salvage committed 98b2964: T2 union surfaces, per-frame tb
  file attribution, scrapy shims (urllib proxy/service_identity/
  tldextract/OpenSSL TLSv1/inspect degrade reorder); gates 417/0, corpus
  233/271 (all fails pre-existing; T104-event-handshake FIXED).
- AFTER: Plan 03 WASM, Plan 04 packaging (uv).
- Ecosystem targets locked (master.md tier table): scrapy T1, pytest T2,
  hermes-agent T3, fastapi T4, comfyui T5 (gates on Plan 10). TEST DATA ONLY -
  never published, no license constraints apply, comfyui included without
  restriction.
- pydantic-core (Rust extension) is the recurring blocker for T3/T4 - strongest
  single case for Plan 10 after basic C-API viability.
