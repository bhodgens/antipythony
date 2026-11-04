# T-10 — threading (gates implementation leaf 02.07 Native threading)

Authorship: written from the spec pack ONLY (`docs/spec/02-control-flow.md`,
`docs/spec/05-builtins-stdlib-v1.md` — the `threading` row of §5.3.2 and the
OUT-of-v1 list §5.3.3, `docs/compat-contract.md` C-14/C-19/C-28 + divergence
register D1/D3/D4, `plans/antipythony/master.md`, leaf 02.07's Expose/Gate in
`plans/antipythony/02-native-backend/orchestrator.md`), per
`plans/antipythony/tests/README.md`. The author has not read any implementation
source under `crates/`.

## Purpose

Gate leaf 02.07 ("threading module on OS threads; object-state thread-safety
audit; GIL-free guarantee"). The leaf has two gate halves:

1. **Corpus threading slice green** — every case in
   `tests/corpus/threading/` must pass ap-conform byte-exactly (stdout,
   stderr, exit) against the reference pins below. The slice covers the
   threading v1 subset of spec 05 §5.3.2 (`Thread`, `current_thread`,
   `main_thread`, `active_count`, `enumerate`, `get_ident`, `get_native_id`,
   `Lock`, `RLock`, `Condition`, `Event`, `Semaphore`, `BoundedSemaphore`,
   `Barrier`, `BrokenBarrierError`, `Timer`, `local`, `TIMEOUT_MAX`-adjacent
   error semantics) plus the C-28 SystemExit-in-thread rule and the C-19/D1
   no-refcount-visibility rule.
2. **Scaling case >= 3x on 4 cores** — `T121-scaling-parallel-cpu.py` is the
   multi-threaded CPU corpus case; the gate is MEASURED by the harness stub in
   `tests/t10_threading/test_t10_threading.py` (section C), never asserted in a
   conformance case (D4).

Determinism is the enemy of this slice: CPython's threading has no pinned
interleavings, so every case is written so its output is a function of the
*program logic* only — results collected into containers, `sorted()` before
print, and synchronization exclusively via rendezvous primitives (Event
set/wait handshake, Barrier crossing, Condition `wait_for` predicate,
queue.Queue sentinel drain, Thread.join). Never via sleep-based timing.

## Testability contract under test

- `ap-conform run --corpus <glob> --impl <cmd>` diffs **stdout bytes, stderr
  bytes, exit code** per case (contract C-24/C-27). Pinned expectations in
  `tests/corpus/threading/expected/` were produced by real
  `/opt/homebrew/bin/python3.14` (3.14.7) runs on this machine — see
  CONTRACT-GAPS G-0 (patch pin) — and additionally each case was run **3×**
  and all three runs were byte-identical before pinning (a flaky case was
  REWRITTEN into rendezvous form, not deleted: corpus never weakens).
- The reference-vs-reference differential through
  `ap-conform run --corpus 'tests/corpus/threading/*.py' --impl <python3.14>`
  is green over the whole slice (proves the harness's per-case timeout of
  10 s default is not hit and the triple contract holds here; run with
  `--timeout 60` in the fixture test for CI margin).
- Until 02.07 lands, the antipythony-side tests in `tests/t10_threading/` are
  `xfail(strict=False)`; the fixture self-checks (reference re-run byte-compare,
  3-run determinism, triple presence, header-citation lint, no-sleep lint,
  refcount-API lint, OUT-of-v1-module lint, scaling-mode stdout-identity check,
  harness self-comparison, scaling plumbing) run and MUST stay green — they
  test the fixtures and the harness, not antipythony.
- Invocation overrides for CI: `AP_CONFORM=<path>` (default
  `target/debug/ap-conform`), `APX_IMPL="<cmd…>"` (impl command line, e.g.
  `target/debug/antipythony run`).

## Run command

```
/opt/homebrew/bin/python3.14 -m pytest tests/t10_threading/ -q
```

From the repo root. Current status: **97 passed, 11 xfailed, 0 errors**
(97 = fixture self-checks incl. 3-run determinism over 18 cases and the
ap-conform self-comparison; 11 = antipythony-side gate params: 1 full slice +
8 spot cases + 1 GIL-free correctness differential + 1 scaling gate).
Full-repo collection (`pytest tests --collect-only`) shows zero collection
errors from this slice.

## Corpus case table (tests/corpus/threading/)

| Case | Cites | Pins (one line) |
| --- | --- | --- |
| T101-start-join-completion-order.py | S-05-4(threading), C-01, C-24 | 8 workers released by one Barrier, joined, per-index results printed sorted — completion order unobservable |
| T102-lock-shared-list.py | S-05-4(Lock), C-01 | shared list appended only under Lock; per-tag counts + per-tag completeness printed after joins |
| T103-queue-producer-consumer.py | S-05-4(Thread), C-01 (queue: see G-2) | queue.Queue 200-item producer/consumer with sentinel; exact multiset + drained qsize pinned |
| T104-event-handshake.py | S-05-4(Event), C-01 | Event readiness announce + go/no-go release; completion names sorted; is_set/clear semantics |
| T105-lock-rlock-semantics.py | S-05-4(Lock, RLock), C-01 | Lock non-blocking acquire/release bookkeeping; `with` on both; RLock depth-3 reentrancy; cross-thread checks sequenced by join |
| T106-thread-local.py | S-05-4(local), C-01, D1 | threading.local per-thread namespace: each worker reads its OWN value; main thread never sees worker attributes (presence check only — no lifetime assertion) |
| T107-daemon-exits-with-main.py | S-05-4(daemon), C-01, C-28 | daemon parked on a never-set Event cannot block process exit; exit 0; daemon's post-wait print unreachable by construction; `started` Event orders daemon-started vs main prints |
| T108-thread-naming-repr.py | S-05-4(current_thread, main_thread, enumerate, active_count, get_ident), C-01 | 3.14 default auto-names `Thread-N (target)`; active_count 1→4→1 through Event rendezvous; repr HEADS only (address/idents masked); get_ident/get_native_id > 0 |
| T109-exception-in-thread.py | S-05-4(excepthook default), S-02-35/37, C-01/24/25/28 | uncaught ValueError in worker → stderr report `Exception in thread <name>:` + threading.py bootstrap frames with 3.14 markers + user frame + summary; exit code stays 0; stdout post-join print intact (see G-1) |
| T110-thread-error-semantics.py | S-05-4(Thread, Lock, RLock, ThreadError), C-01 | exact RuntimeError texts: release-unlocked, foreign RLock release, join-before-start, double start, daemon-set-after-start; ThreadError ⊂ RuntimeError |
| T111-condition-rendezvous.py | S-05-4(Condition), C-01 | Condition wait_for producer/consumer handshake (closed predicate) + entered/notified micro-protocol with no lost wakeup |
| T112-semaphore-bounded.py | S-05-4(Semaphore, BoundedSemaphore), C-01 | permit counting via blocking=False probes; BoundedSemaphore over-release exact `ValueError: Semaphore released too many times` text (from joined worker); plain Semaphore over-release allowed |
| T113-barrier.py | S-05-4(Barrier, BrokenBarrierError), C-01 | Barrier(4) all-parties crossing (pre/post sets sorted), broken False, parties/n_waiting after reset; solo `wait(timeout=0.05)` → BrokenBarrierError with pinned empty str, broken True (timeout *result*, not an ordering — see determinism rule 7) |
| T114-system-exit-in-thread.py | C-28, S-05-4, C-01 | SystemExit(5) in worker ends THAT thread only; main joins, prints, process exits 0 |
| T115-timer.py | S-05-4(Timer), C-01 | Timer ⊂ Thread; zero-delay fire+join; cancel-before-start ⇒ body never runs; join of never-started timer → exact RuntimeError text |
| T120-gil-free-cpu-probe.py | S-05-4(Thread), C-01/19/24 | GIL-free CORRECTNESS probe: 4 independent CPU threads byte-equal the sequential baseline (pinned); speed NOT asserted (D4) |
| T121-scaling-parallel-cpu.py | S-05-4(Thread), C-01, D4 | scaling-gate workload: default mode prints pinned result-only bytes; `T10_MODE=serial\|parallel` prints one timing line to STDERR (never in the triple) for the measured-not-asserted harness |
| T122-no-refcount-visibility.py | C-19/D3, D1, C-14, C-01 | threaded allocate/drop loop with no `__del__` and no refcount API: output byte-identical across runs/impls; nothing about collection timing observable |

18 cases, all ≥ the leaf's "corpus threading slice"; every case has an
expected triple from real reference runs (3× byte-identical).

## Determinism rules applied per case

1. **Collect-then-sort**: every container written from ≥2 threads is reduced
   only after all writers are joined, and printed via `sorted()` (T101, T102,
   T104, T106, T111, T113) or in a FIXED key order (T120, T122 iterate
   `range(n)` keys, not dict order).
2. **Rendezvous, never sleep**: cross-thread *ordering* uses Event set/wait
   (T104, T107, T108), Barrier parties (T101, T113), Condition wait_for
   (T111), queue sentinel drain (T103), or start/join sequencing (T105, T110,
   T112, T114, T115). A `test_no_sleep_based_timing_in_corpus` lint bans
   `sleep(` in every case source.
3. **Locked writes for shared containers** (T101, T102, T104, T106, T113,
   T120, T122): appends happen under a Lock so the *container* contents are a
   deterministic multiset even before the sort (belt and braces — antipythony
   may one day expose unsynchronized-append races as real corruption; the
   corpus pins programs whose result is well-defined under either memory
   model).
4. **Address/id suppression**: `Thread.__repr__` contains a tid and possibly
   an address; only the HEAD (`split(",")[0]`) is printed (T108). `get_ident`
   / `get_native_id` results are printed as `> 0` booleans, never values.
5. **Inter-thread prints ordered by handshake**: T107's daemon print vs main
   prints is ordered by a `started` Event, so the stdout stream is a pinned
   sequence, not an interleaving roll.
6. **Exception-in-thread report (T109)**: CPython emits the thread excepthook
   output synchronously from the dying thread before `join()` returns, and the
   process exit code is unaffected (C-28 shape) — the bytes are deterministic
   *given* the join; pinned live. Content gap in G-1.
7. **Timeout appears only where the RESULT is deterministic**: T113's
   `wait(timeout=0.05)` on a Barrier(2) with no second party always raises
   BrokenBarrierError regardless of machine speed (the timeout expiring IS
   the outcome); we pin the exception + `broken=True`, never "it took about
   50 ms". No case asserts wall-clock ordering.
8. **3-run pin gate**: before writing any triple, the case ran 3× under
   `/opt/homebrew/bin/python3.14` and all three runs matched byte-for-byte on
   stdout+stderr+exit; two early drafts that could have raced (a lock
   double-release check, a lost-wakeup Condition micro-check) were rewritten
   into the sequenced/predicated forms above. The pytest suite re-checks this
   (`test_case_is_three_run_deterministic`).

## Scaling-gate handling (02.07: "scaling case >= 3x on 4 cores")

- The workload is corpus case **T121**: 4 CPU-bound seeds (pure int modular
  arithmetic, no I/O, no allocation-heavy shared state); default run prints
  only the pinned `result bytes only; scaling measured by harness` line plus
  an internal assertion that parallel == sequential results.
- `T10_MODE=serial` / `T10_MODE=parallel` add ONE stderr timing line; the
  harness (`_measure`) runs each mode best-of-3 wall time and computes
  speedup. Timings never enter the conformance triple (D4).
- **Measured-not-asserted for the reference**: a green plumbing test runs both
  modes under python3.14 and records the numbers only. This is required: the
  local homebrew CPython 3.14.7 is **GIL-enabled** (verified live:
  `sys._is_gil_enabled()` → True), so its 4-thread CPU speedup is ≈1.0x
  (measured here: 0.997x). Asserting ≥3x against CPython would assert CPython
  violates its own GIL. The ≥3x assertion exists ONLY as
  `test_scaling_gate_3x_on_4cores_antipythony`, `xfail(strict=False)` until
  02.07 ships its GIL-free runtime.
- **Core-count guard**: the gate test is `skipif(os.cpu_count() < 4)` with the
  host count in the reason string. This authoring host reports **24 logical
  cores** (`os.cpu_count()`, `sysctl hw.ncpu` — Apple Silicon), so the gate
  ran un-skipped (as xfail) locally. CI on 2-core boxes skips with an
  annotated reason instead of false-failing; a 4-core box runs it.
- Reviewers of the 02.07 gate should treat a *permanent* xfail on the gate
  test with reason "awaits 02.07" as red; the leaf completes when it turns
  XPASS (then promote to strict) — never by lowering the 3.0 multiplier.

## CONTRACT-GAPS

- **G-0: patch pin vs live reference (C-01).** C-01 pins conformance to
  CPython **3.14.0**; the only binary here reports 3.14.7 (`-VV`). Same
  provenance gap as T-05 G-0: all triples here are provisional and must be
  diff-verified against a 3.14.0 build before the 02.07 gate is certified.
  Threading bootstrap internals (threading.py line numbers in G-1) are the
  most patch-sensitive bytes in this slice.
- **G-1: thread-excepthook stderr embeds the reference's own threading.py
  frames + absolute install path.** T109's pinned stderr contains
  `File "<abs path>/python3.14/threading.py", line 1082, in _bootstrap_inner`
  and `line 1024, in run` — i.e. CPython's stdlib *source* frames with a
  machine-specific absolute path and version-specific line numbers. Byte-
  exactness here cannot mean "antipythony ships an identical threading.py at
  the same path" (impossible: no GIL bootstrap, different stdlib tree). The
  compat contract's traceback normalization (spec 02 §Corpus anchors: "the
  harness compares after normalizing the two invocations to the SAME path
  text") covers the USER file path but NOT stdlib frames. Options (impl/spec
  must choose; test track will re-pin to the decision, never silently):
  (a) antipythony's threading bootstrap frames are *suppressed* — divergence,
  needs a register entry; (b) ap-conform extends its path normalization to
  stdlib frame lines + line numbers; (c) T109 is re-split so only the header
  line + summary line + user frame are gated. The spot gate for T109 is in
  the xfail param list precisely so this cannot pass by accident.
- **G-2: `queue` is not in spec 05 §5.3.2.** The producer/consumer case T103
  needs a rendezvous FIFO; `queue.Queue` is the only stdlib shape that pins
  "exact multiset received" without sleeping. Spec 05 lists it in NO row (IN
  or OUT). Filed for a spec-05 PR: either add `queue` (`Queue`, `SimpleQueue`,
  `PriorityQueue`, `LifoQueue`, sentinel protocol) to the §5.3.2 table as IN
  for 02.07's slice, or declare it OUT and T103 must be *rewritten onto a
  Condition-backed internal queue* (a corpus PR, since the case itself stays).
  Until then T103's header cites the gap inline.
- **G-3: spec 05 threading row names `TIMEOUT_MAX` but no clause pins its
  VALUE or the float-coercion of `wait(timeout=float('inf'))`.** Measured
  `threading.TIMEOUT_MAX == 9223372036.0`; a value-bearing constant print was
  deliberately kept OUT of the corpus (a rebrand of the same number by impl
  would be a gate we do not want to certify without a clause). Gap: spec 05
  should either add a clause pinning TIMEOUT_MAX or mark it corpus-
  prohibited like `time` values. Not used in any case.
- **G-4: "exception in thread printed … in CPython format" (task brief) vs
  the format spec's silence.** Spec 02 S-02-35 pins the *main-thread*
  uncaught-exception report; the threading excepthook report
  (`Exception in thread <name>:` header, S-05-4's `excepthook`/`ExceptHookArgs`
  names) has NO format clause anywhere in the spec pack. T109 pins the
  measured live bytes as provisional ground truth; a clause (proposed
  `S-05-24` or a spec-02 §6 extension) is owed so the header text + per-
  thread-name rule become normative. Cite in the PR against docs/spec.
- **G-5: default thread naming `Thread-N (target_fn)` is 3.14-specific
  formatting** (the `(fn)` suffix; and the counter starts per-name-class).
  T108 pins `['Thread-1 (noop)', 'Thread-2 (noop)', 'Thread-3 (noop)']` as
  measured. No spec clause owns this string shape; same remedy as G-4 (the
  naming rule is observable via `enumerate()`/`active_count()` and SHOULD be
  pinned by clause, not only by triple bytes).
- **G-6: daemon-thread stdout flush ordering at interpreter exit is D1-adjacent
  but unregistered.** T107 avoids it (the daemon's only print happens before
  the main thread's `started.wait()` returns), but the contract register
  (D1–D4) says nothing about output a daemon writes *concurrently with* exit.
  No corpus case may rely on it; recommend a register note so future authors
  do not pin such interleavings.
- **G-7: no refcount-visibility probe can be *positive*.** C-19/D3 make
  refcounting unobservable, and prohibit the introspection APIs, so T122 can
  only pin that a churn-heavy threaded program is byte-stable. That is
  consistent with (not a relaxation of) C-14: the exception surface is
  unchanged by threading (no new MemoryError paths in the pinned workload).
  Recorded so reviewers see the probe was written to the letter of D3 (no
  `getrefcount` anywhere — enforced by lint).

## xfail policy

Same as T-05: all antipythony-side tests are `xfail(strict=False, reason=
"awaits 02.07 native threading (antipythony impl side)")` — 11 params (full-
slice ap-conform run, 8 per-channel spot cases, the T120 GIL-free correctness
differential, the T121 ≥3x scaling gate). The scaling *plumbing* test and every
fixture self-check stay green today. Until 02.07 lands the suite shows xfail,
zero hard failures = green for this test leaf. After landing, XPASS is
expected; any FAIL is a 02.07 implementation bug — fix the impl, never the
corpus (corpus rule / AGENTS.md).
