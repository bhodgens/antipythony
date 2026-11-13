# antipythony vs CPython 3.14 — High-Level Benefits Summary

Measured-gates synthesis, 2026-11-13. All numbers are recorded gate results, not projections.
Machine: Mac M2 Ultra 24-core, macOS 26.7 aarch64. Trunk stable at 0e313536+.
CPython reference: 3.14.0 (codecs ground-truthed against 3.14.7).

## 1. Executive summary — what is delivered today

- **Real JIT speedups, conservatively measured**: 2.07x geomean speedup (adaptive vs
  forced tier-0) on an arithmetic/string function-shaped slice; floor was 1.5x.
  Per-probe worst-case: int-sum 3.20x, int-mul 2.65x, str-len 1.68x, str-concat 1.24x.
  (docs/tier1-bench.md)
- **The GIL is gone**: threading scaling gate T121 measures 3.31x/3.30x best-of-3 in a
  quiet window (gate: >=3x on 4 cores); the @xfail mark was flipped, the >=3.0
  assertion untouched. (PROGRESS-0209b.md)
- **Native single-file binaries**: `antipythony build app.py -o app` emits a 17.7 MB
  Mach-O arm64 executable linking only libSystem, with startup at 0.69x of the JIT
  `run` path; the T-09 30-case conformance slice passes through the binary 30/30.
  (docs/aot-binary-gate.md)
- **Unboxed integer lists cut list memory ~32-36%**: 2M-element int lists drop
  16B/elem to 8B/elem (30.4-31.8 MB vs 47.7-50.1 MB max RSS for list(range(2_000_000))).
  (PROGRESS-0209c.md)
- **Byte-exact conformance is the exit gate for everything above**: every gate ran
  differentially against live CPython 3.14 — outputs, stderr, and exit codes must
  match before a feature counts as landed.
- **Real third-party programs run**: scrapy 2.19.0 imports and crawls with items.jsonl
  byte-identical to CPython (T1, docs/t1-gate.md); pytest 9.1.1 runs a suite to a
  verdict identical with CPython under default config (T2, AGENTS.md status block).
- **stdlib surface is fully populated for imports**: all 53 spec-05 modules resolve
  (31 native + 22 vendored), zero missing — the gap is behavior depth inside modules,
  not presence. (.hermes/audits/2026-11-11-0208-stdlib-coverage-audit.md, landed 0ed927fa)
- **Measured CPython-exact CJK codecs**: 1056/1056 encode+decode differential cases
  across 24 codecs. (AGENTS.md status block, wave a8841dc)

## 2. Performance benefits

### Tier-1 JIT (leaf 02.05h) — 2.07x conservative geomean

- Estimator: `min(tier-0)/max(adaptive)` per run — the T-08 G-3 lower-bound rule that
  counts noise AGAINST the speedup. Geomeans across 3 runs: 2.13 / 2.07 / 2.05
  (median/median: 2.21 / 2.19 / 2.20). Gate floor 1.5x.
- Conditions: RELEASE ap-run-eval build at main = 9d0a9f4 (02.05a-g all landed); 3 runs
  x 9 timed reps per mode per program, warmup first; wall-clock around the whole
  process, so the numbers INCLUDE ~5 ms process startup and JIT compile time paid
  inside the warm calls.
- Wins decompose two ways: typed-region fast paths (i64-smi guards) remove the boxed
  binop round-trip on int loops (3.20x); the compiled baseline twin alone is worth
  ~1.7x where tier-1 declines the body (string ops).
- Correctness held while speeding up: tier_mode_differential 3/3 and osr_gate 2/2 on
  the gate tree; an ad-hoc 287-file corpus sweep found 279/287 byte-identical across
  tier modes, with the 8 exceptions being mode-independent nondeterminism (pointer
  reprs, thread scheduling).

### No-GIL scaling (leaf 02.09b) — T121 >=3x GREEN

- Best-of-3 quiet window: serial 0.992s / parallel 0.300s = 3.31x and 0.987/0.299 =
  3.30x; the pytest harness form of the gate measured 3.22x/3.30x in the same window.
  Threading slice T101-T115 + T120/T122 stayed byte-exact (18/18 vs pinned triples)
  through the flip.
- How: GilState/acquire_gil/GilGuard deleted (54 call sites); Gc<T> on Arc refcounts;
  21 class registries moved to per-interp rows; PyDict/PySet/PyList/PyObject/
  Interpreter interiors under Mutex/RwLock; GC collector made process-global with a
  mutator census and per-worker registration channels. An independent audit verified
  the harness fix (the old wall-clock shape arithmetically capped the observable ratio
  at ~1.5x) and its findings were reconciled before merge.
- Conditions: quiet window (<4 load) required — trunk re-verifies under external load
  spikes (meept go-test lane, load 9-11) measured 2.53-2.96x; quiet-window re-verify on
  the fully merged trunk is still a PENDING item in the plan tree (best under residual
  load so far: 2.92x).

### Unboxed list storage (leaf 02.09c) — memory

- PyList of Python ints stores a raw Vec<i64> (8B/elem) instead of boxed 16-byte Value
  slots; one-way demotion to boxed on the first non-Smi write; list(range(N))
  materializes directly into raw storage.
- Measured (debug builds, /usr/bin/time -l max RSS, ours vs the 02.09b trunk binary
  side-by-side): list(range(2_000_000)) 30.4-31.8 MB vs 47.7-50.1 MB (~-36%);
  append-loop 2M ints 34.5 MB vs 50.7 MB (~-32%).
- No semantics traded: method_surface_differential 34/34, gc slice 20/20, and the full
  corpus 248/287 with zero new failures vs the trunk baseline. The GC fence invariant
  (unboxed = zero internal edges, still registered) is pinned by a unit test.

### AoT startup (leaf 02.06)

- Startup 0.69x of the JIT run path: 4.0 ms vs 5.8 ms, min-of-5 (soft gate was <=2x).
  The binary is a normal executable: argv passthrough, cwd invariance, and PATH
  isolation (no python3.14 visible) all verified.

## 3. Utility and compatibility benefits

- Byte-exact conformance standard: differential comparison against live CPython 3.14.0
  (stdout/stderr/exit code) is the exit gate for every leaf; ap-conform --impl-binary
  extends it to compiled binaries. Post-flip merged-trunk gates: workspace 540 passed /
  0 failed; T10 threading slice 105 passed / 2 xfail; xtask notices --check OK (345 deps).
- stdlib import coverage 53/53: the spec-05 module list resolves entirely — 31 native
  modules + 22 vendored, zero missing. The honest caveat is depth: the tier-0 ap-conform
  stdlib behavioral slice is 30/48 (16 fail + 2 timeout), adaptive mode is
  byte-identical, and the binary is 29/48. (.hermes/audits/2026-11-11-0208-stdlib-
  coverage-audit.md, landed 0ed927fa)
- Ecosystem tiers T1 and T2 green: scrapy 2.19.0 crawls file:// and produces
  byte-identical items.jsonl (docs/t1-gate.md, ~25 blockers cleared); pytest 9.1.1 runs
  a suite under default config (cacheprovider enabled) with a verdict identical to
  CPython — 1 failed / 1 passed / 1 skipped, rc=1, zero INTERNALERROR (W112 campaign).
- AoT binary artifact: one command produces a self-contained native executable — no
  interpreter process, no script file at runtime; the T-09 pinned slice runs through
  the binary 30/30, a GC-heavy 20M-allocation probe is green in-binary, and panics are
  caught (catch_unwind -> one line + rc 134, never a raw Rust panic dump).
- Native speed without abandoning CPython semantics: tier modes are observationally
  equivalent by gate (tier_mode_differential 3/3), and lowering-rejected functions fall
  back to the tree-walker loudly at build time ([AOT-BUILD] notice) rather than changing
  behavior.
- CPython-exact text pipeline: 24 CJK codecs at 1056/1056 differential parity, unblocking
  scrapy's response.text path.

## 4. Architectural benefits that enable future work

- A true no-GIL foundation: concurrency is built on Arc refcounts, per-object
  fine-grained locks, a process-global GC collector with a mutator census, and
  safepoint-style contention aborts — not on a lock around the interpreter. The 02.10
  Go-scheduler follow-up is unblocked by the flip.
- MMTk-ready GC seam (docs/gc-mmtk-swap.md): the collector already sits behind ap-gc
  public functions and a collector-agnostic interpreter bridge; the migration plan maps
  the registry onto MMTk enumeration and weakref liveness onto ReferenceProcessor, with
  generational/copying plans as a post-tier-1 one-line plan flip.
- Shared lowering across backends: JIT tier-0, tier-1, and AoT all lower through the
  same cfg-to-CLIF stage; slot plans are shared (a unit test pins JIT/AoT parity), so
  backend coverage grows together and fallbacks are the documented, loud exception.
- Background compiler thread model (02.05e/f): a TierUpDriver with CAS install,
  snapshot-at-enqueue, pin-until-retire, and boxed OSR end-to-end (back-edge polls,
  OSR-BY-CALL entry) — the concurrency machinery for future typed-OSR and
  for-loop/generator OSR is in place.

## 5. Honest limitations — what is NOT yet better than CPython

- The C-extension wall: pydantic-core (a Rust extension) is the recurring blocker for
  tiers T3 (hermes-agent) and T4 (fastapi); real C extensions remain refused (APX-CEXT).
  Plan 10 is the hail-mary path.
- Adaptive-mode traceback linenos are imprecise: JIT frames attribute to the def line
  (filed alongside the 02.05h gate).
- Debug-build single-thread tax from per-object locks: post-flip fence probe ~2.9 s/iter
  vs 2.3 pre-flip (+26%) in debug; release unaffected — but it is a real cost until
  tier-1 work recovers it.
- musl static-linux builds unverified: no musl toolchain on the host; --target linux-musl
  is implemented and fails loudly naming the missing toolchain. The 15 MB size gate is a
  musl bound, unmeasurable here — macOS is report-only for size.
- stdlib behavioral depth: tier-0 ap-conform slice 30/48 (16 fail + 2 timeout); ~16 named
  surface gaps (Counter repr, functools CacheInfo, ast.unparse, typing repr/origin, an
  enum mangling bug, pprint, argparse, warnings attribution) — a fix wave is in flight;
  stdlib .py payloads are not embedded in AoT binaries yet.
  [UPDATE 2026-11-13: the iter(<genexpr>) wedge listed by the earlier audit was FIXED
  this session — three re-entrant mutex self-deadlocks + genexp eager-clause semantics
  (86fa7842, 0e313536); s030_copy and s044_pathlib un-hang, verified byte-exact on trunk.]
- Tier-1 coverage caps: a baseline-JIT emit bug (builtin Call with a local-Name arg
  feeding a BinOp in a while loop fails Cranelift verify) transparently falls back to
  tier-0 for that shape; float-arithmetic loops decline tier-1 by design of the current
  hotness gate; module-level For bodies are outside the cfg stage (tree-walk under JIT
  and AoT, loudly).
- T121 noise sensitivity: scaling numbers require a quiet window; re-verification under
  system load measured 2.53-2.96x (meept sibling go-test lane; best under residual load
  2.92x).
- Two-tier bench scope: the 2.07x gate covers an arithmetic/string function-shaped slice;
  module-level loops never reach the JIT dispatch arm (the older 0.96x corpus report
  measured startup noise, not tier-1).

## 6. One-line takeaway

antipythony already delivers, with measured gates, what CPython 3.14 does not: a
free-threaded runtime at 3.3x threading scaling, a 2.07x-(conservative) JIT tier that
stays byte-exact with CPython, ~34% lighter int lists, one-command native binaries that
start 1.45x faster — all held to byte-identical conformance against CPython 3.14, with
the C-extension wall and stdlib depth as the clearly-filed remaining frontier.
