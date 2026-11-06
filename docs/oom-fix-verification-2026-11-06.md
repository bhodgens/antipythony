# OOM Fix Verification — Wave 1 + Wave 2 (2026-11-06)

Independent adversarial verification of the wave-1 and wave-2 OOM fixes.
Verifier: separate agent, no shared probes with the fixers (all probes written
fresh from the bughunt doc descriptions; sources in `/tmp/oomverify/`).

## Method

- **Waited** for both fixer waves to land: wave-1 = 7eb65a4 (F2), 9ee848c (F1),
  090ad43 (G1+F5), eb6f7b4 (G3), 72b6fb5 (F3 + co-landed wave-2 hunks M5/G9/M6,
  G2 dispatch arm), f7ea3a3 (docs); wave-2 = eece027 (G2), c80550c (G4),
  c96f41c (G8 + F5 Drop-hook TLS fix), 67d3e09 (G5+G6), 86c4cdc (G7+G10+F4),
  0f3008e (docs).
- **RSS probes**: own driver (`/tmp/oomverify/probe_rss.py`, fork + wait4
  rusage maxrss, child stdout to /dev/null). Each finding probed at ≥2 sizes
  against `target/debug/ap-run-eval`. **Pre-fix baselines captured on the
  pre-wave tree (4709a20) BEFORE rebuilding**; post-fix numbers on the final
  tree (0f3008e). NOTE: macOS `ru_maxrss` is bytes; the bughunt docs quoted KB
  from Linux-style harness — magnitudes compared consistently in MB below.
- Env caps on every run: `AP_MAX_RSS_GB=2 AP_PROBE_MAX_RSS_GB=2
  AP_PROBE_TIMEOUT_SECS=240`. Serial cargo (pgrep preflight); no product code
  touched; forbidden files untouched.
- **Regression gate**: pytest t05/t06/t07/t10/t11 + ap-gc cargo tests + 8
  ap-run differential suites (re, re_fuzz, gc, lxml, import, method_surface,
  name_resolution, classes) + spot semantic probes (lru identity, re.compile
  identity, intern TypeError, generator traceback).

## Verdict table

| ID | Fixer claim (commit) | My measurement (pre → post) | Verdict |
|----|---------------------|-----------------------------|---------|
| **F1** fiber stack 64MB/gen | Reservation 64→8MB, committed RSS unchanged (9ee848c) | 21.5/54.6/98.6MB @N=50/200/400 held gens → 21.5/54.5/98.5MB post. Slope identical (~258KB/gen committed) — fixer honestly claimed this; the leak class (VA reservation compounding) is bounded, committed-RSS slope is NOT improved | **VERIFIED-WITH-NOTES**: reservation fixed; committed-RSS per suspended gen (~258KB) remains — the doc's "8k gens ≈ 2GB" scenario is now ~2GB VA but still ~2GB committed RSS |
| **F2** weakref+callback never pruned | prune dead entries + id-keyed drain (7eb65a4) | 54.4MB/7.9s @20k; 146MB/62s @60k → 15.2MB/2.1s @20k; **16.4MB/9.5s @100k FLAT** (was 188MB/177s per doc) | **VERIFIED** (t07 gc 131 passed) |
| **F3** StringIO O(n²) write | pop+try_unwrap+extend in place (72b6fb5) | 19.1MB @20k; 250MB @100k → 13.3 @20k; **22.7 @100k; 32.8MB @200k** (was 485MB @200k) | **VERIFIED** |
| **F4** setrecursionlimit → SIGABRT | stack-budget hard cap in enter_call (86c4cdc) | pre: rc=-6 @lim5100/depth5000 → post: **clean RecursionError at 5100/20000/40000**; default-limit behavior unchanged (RE@1000) | **VERIFIED-WITH-NOTES**: in debug builds the hard cap is ~1024 (256MB/256KB), so raising the limit does NOT actually permit deeper recursion than default in debug — safe (RE not crash) but "setrecursionlimit(40000) works to 39k" is false on debug builds; release (50KB/frame) caps at ~5242 |
| **F5** genexp/iterator registry growth | Drop-hook + prune_dropped_iters (090ad43, c96f41c) | 15.7/33.5MB @20k/100k → 16.4/35.2 @20k/100k; 143.9MB @500k — my genexp-pull probe slope roughly unchanged | **VERIFIED-WITH-NOTES**: break-path (G1) and abandonment are fixed; the sum(x*2 for x in range(5)) single-pull shape still shows small linear growth (~216KB/1k iters) — residual, low severity |
| **G1** break pins iterator + full clone | Break/Escape prune + SharedList/Tuple (090ad43) | pre: **1.63GB @1k breaks; rc=-6 @20k** → post: **11.7MB @100k FLAT** | **VERIFIED** — biggest single win; also stops the per-iter() container clone |
| **G2** sre MATCH/PAT tables | 1024-cap ledger + RE_CACHE 512 (eece027) | drop-path flat 10.2MB @200k; `re.compile(p) is re.compile(p)` True; my original probe was address-recycle-masked pre-fix (10.4MB flat both) — fixer's 33/115/217MB held-match numbers not independently reproduced, but bounded-table mechanism verified in diff | **VERIFIED-WITH-NOTES** |
| **G3** lxml DOCS push-only | Weak slots + 64-doc LRU pin window (eb6f7b4) | 55.3MB @50k → **11.5MB @200k FLAT** (pre 55MB @50k rising) | **VERIFIED** (lxml_differential 24/24) |
| **G4** nested def/lambda per-execution | NESTED_DEDUPE keyed (AST node, defining frame Rc) (c80550c) | **pre 360MB @100k / 1.78GB @500k → post 372MB @100k / 1.87GB @500k — NOT IMPROVED** for the doc's actual trigger (def/lambda inside a function called per item: fresh frame per call ⇒ fresh key ⇒ appends per call). Same-frame re-execution IS fixed (10.8MB flat @500k). Lambda same-frame flat. | **FAILED-VERIFICATION** (partial fix) — evidence: `crates/ap-run/src/frame.rs:482` keys on `Rc::as_ptr(defining)` (runtime frame address, new per call); repro `/tmp/oomverify/g4c_freshframe.py`: `def outer(): def helper()…; return helper()` looped 200k ⇒ 551MB. Fixer's probe (`g8c_ctl`) exercised the same-frame shape only |
| **G5** CELL_BACKING per-access | CELL_WRAPPER_CACHE keyed (slot, name) (67d3e09) | 11.1MB flat pre (masked) → 11.0-11.2MB flat @1M post; wrapper-identity semantics checked vs CPython (stable per access) | **VERIFIED** |
| **G6** stdout whole-run buffer | 1MB threshold drain + `stdout_flushed_prefix` re-prepend (67d3e09) | buffer itself bounded ✓, but 18.1/45.3MB @100k/500k → 19.0/46.3MB, **147.6MB @2M lines** — the drained prefix + exit-time `clone()` keep ~2× the FULL output resident (67MB output → ~134MB at exit, matches measurement). "1GB of print" still costs ~2GB RSS at exit, ~1GB during the run | **FAILED-VERIFICATION** (design preserves byte-identical RunOutcome at the cost of the fix's purpose) — evidence: `crates/ap-run/src/frame.rs:447` keeps every drained byte in `stdout_flushed_prefix`; `crates/ap-run/src/interp.rs:153` clones it at exit; `ap_run_eval` never reads `RunOutcome.stdout` (only exit_code), so for the production binary the prefix is pure waste |
| **G7** WORKER_REG_IN unbounded | drain on gc.collect + run tail (86c4cdc) | no-collect path: 303/1470MB @200k/1M → **304/1473MB UNCHANGED**; with `gc.collect()` in the loop: **21.9MB @200k / 23.5MB @1M bounded** | **VERIFIED-WITH-NOTES**: fixer's own doc admits "threading not probed". The fix bounds the channel ONLY for workloads that call gc.collect(); a spawn-once thread + pure-alloc work with no collect still grows until exit (bounded only by the new run-tail drain, i.e. still whole-run growth). Doc's trigger scenario is only partially covered |
| **G8** exhausted generators never retire | drop entry when fiber.done() (c96f41c) | churn 11.2MB flat @500k post (pre 11.2 masked by fiber churn; fixer's 47/188/359MB → 11.4-11.6MB not contradicted); G001 byte-identical; exhausted-throw semantics verified | **VERIFIED** |
| **G9** lru_cache maxsize=0 | `matches!(maxsize, Some(m) if m>0)` gate (72b6fb5) | `maxsize=None` caches (CALLS=N=50k); `maxsize=0` recomputes (CALLS=2N) — both before AND after via public API; native gate now skips insert for Some(0)/negative | **VERIFIED** — CPython semantics preserved both ways |
| **G10** tb_frames index-0 insert leak | insert at GEN_TB_BASE (resume depth) (86c4cdc) | pre: 108/398/1514MB @100k/300k/1M → post: **11.6/11.8/11.7MB FLAT through 1M**; traceback of caught gen exception intact | **VERIFIED** — second-biggest win |
| **M6** LINECACHE | Rc lines + mtime checkcache + 64MB cap (72b6fb5) | 12.1MB flat @1M getlines (was flat pre-fix too — cache-keyed by path); checkcache implemented (was no-op) | **VERIFIED** |
| **M8** sys.intern non-str | type-gate TypeError (090ad43) | `sys.intern(42)` → TypeError "intern() argument must be str, not int" — byte-identical to CPython | **VERIFIED** |

## Skipped / not addressed (fixers' own disclosure + my check)

- **M1-M4** (threading LOCALS/THREADS/OBJ_MIRROR/PRIMS): NOT touched — disclosed
  in wave-2 doc (threading.rs sibling-WIP avoidance). Silent-skip NOT hidden —
  documented by fixer, confirmed by me (no threading.rs in any wave commit).
- **M5** NAMED_CALLABLES: addr-dedupe landed (72b6fb5) — not independently
  probed (bounded-by-live-callables argument sound in diff review).
- **M7** WEAK_TOKENS/FINALIZERS: covered by F2 (verified above).
- **M10 FILE_SRC, M11 XPATH_OBJECTS, M12 PENDING_GEN_ERROR**: left for
  follow-up per wave-2 doc (M11 explicitly).
- **Correctness bugs 1/2** (classes.rs kwargs, super_stack double-pop):
  untouched — FORBIDDEN files per task rules; still open.

## Regression gate (corpus never weakened)

- pytest t05+t06+t07+t10+t11: **898 passed, 2 skipped, 155 xfailed, 78 xpassed,
  0 failed**. The 78 xpassed are ALL pre-existing stale `awaits 02.07/02.08`
  ledger markers (checked with -rX; 73 in t10/t11 alone; within the 74-entry
  XPASS_CEILING convention; none in the fixed areas' semantics).
- ap-gc cargo: **14+9 passed, 0 failed**. ap-run lib: 4 passed.
- Differential suites (ap-run): classes 29/29, gc 2/2, import 6/6, lxml 24/24,
  method_surface 34/34, name_resolution 24/24, **re 42/42, re_fuzz 4/4** —
  **165 passed, 0 failed**.
- Semantics spot-checks: lru maxsize=None caching + maxsize=0 no-caching;
  re.compile identity dedup; intern TypeError; generator-raise traceback intact;
  RecursionError at default limit unchanged.

## Code-review notes (behavior changes beyond the fix)

1. **G1 SharedList/SharedTuple** change iteration aliasing: append-during-
   iteration is now visible to the iterator (snapshot before). Comment in diff
   acknowledges; matches CPython's "list mutation during iteration" UB space,
   but IS a semantics change for `for x in lst: lst.append(...)` — differential
   suites green; flag for byte-exact phase.
2. **F2 prune by target_addr**: `fire_dead_weakrefs` prunes ALL tokens whose
   target_addr matches a dead entry. If an address is recycled by a NEW live
   object in the same drain window, a live token could be over-pruned. Window
   is narrow (per-drain), GC-heap addresses are stable while alive — risk
   assessed LOW, but note it.
3. **F4 hard cap** overrides an explicitly-raised recursion limit (debug ~1024,
   release ~5242). CPython 3.14 also stack-caps (its C-limit), so directionally
   compatible; document the debug floor for deep-recursion tests.
4. **G4** — see FAILED-VERIFICATION; the fix as landed also grows
   NESTED_DEDUPE itself unboundedly for fresh-frame-per-call (new key per call,
   never evicted) — the fix ADDS a leak table on the path it didn't fix.
5. **G6** — `stdout_flushed_prefix` defeats the fix for RunOutcome consumers;
   suggested repair direction (not applied per task rules): make the prefix
   capture opt-in (only when the caller actually consumes RunOutcome.stdout),
   or stop keeping it when fd-1 write succeeded and the driver ignores the
   outcome buffer.
6. **G7** — drain hooked into `gc.collect` dispatch only (`gcsupport.rs:630`
   arm); `gc.collect_gen` / other collect entries not hooked (single call site
   checked).

## Bottom line

- **VERIFIED**: F2, F3, G1, G3, G5, G8, G9, G10, M6, M8 (10)
- **VERIFIED-WITH-NOTES**: F1, F4, F5, G2, G7 (5)
- **FAILED-VERIFICATION**: G4 (fresh-frame-per-call path still leaks ~2.7KB/
  call, plus NESTED_DEDUPE itself accumulates), G6 (byte-identical-outcome
  design keeps ~2× full output resident; residual 64B/line slope)
- Regression: **0 failures** across 5 pytest suites + 8 differential suites.
- Skipped (disclosed): M1-M4 threading tables; M10/M11/M12 follow-ups.

## REPAIR (post-verification) — 2026-11-06, same day

Repairs of the two FAILED-VERIFICATION verdicts above, per this doc's own
fix directions. Env caps on every run: `AP_MAX_RSS_GB=2 AP_PROBE_MAX_RSS_GB=2
AP_PROBE_TIMEOUT_SECS=240`; serial cargo (pgrep preflight); forbidden files
(crates/ap-run/src/classes.rs, crates/ap-run/src/ast_bridge.rs) untouched;
no push.

### G4-repair — nested-def dedupe keyed on code-object identity (44c016a)

**Approach.** `NESTED_DEDUPE` key is now
`(defining frame scope_path, def/lambda source line, AST-node addr)` — all
stable across calls of the same enclosing function, i.e. our code-object
identity (the wave-2 fix keyed on the runtime frame `Rc` pointer, fresh
every call, so it deduped nothing). The def source line rides the frame in
a new `Frame.def_line: Cell<usize>`, written by the `Stmt::Line` marker
that precedes every LocalDef/lambda in lowered blocks. Table bounded FIFO
at 1024 entries (`NESTED_DEDUPE_CAP`, mirrors `SRE_TABLE_CAP`) with an
insertion-order ledger. Per-call function OBJECTS are still minted fresh
(CPython semantics — only the code/AST preparation is shared).

**Safety gate.** A def/lambda with frees, nonlocals, default exprs, or
decorators BYPASSES the dedupe (fresh `FnEntry` + `defining_frames` entry
per call = exact wave-1 semantics). Rationale: sharing one entry across
activations would merge distinct calls' closure cells — found empirically
during repair (make-returned closures read the wrong cell after later
calls; `[make(i) for i in range(4)]` produced `0 3 2 3` instead of
`0 1 2 3`) and fixed by the gate. DEDUPED (closure/default-free) entries
skip the `defining_frames` re-pin — nothing reads the frame for them.

**Before/after slope** (repro `/tmp/oomverify/g4c_freshframe.py`, the
verifier's own probe, `probe_rss.py` fork+wait4, debug build):

| Probe | pre-repair | post-repair |
|---|---|---|
| g4c_freshframe @100k | 372MB (rising) | **11.0MB** |
| g4c_freshframe @200k | 551MB | **11.2MB** |
| g4c_freshframe @400k | — (would exceed caps) | **11.1MB** FLAT |
| g4_nesteddef @100k/200k | rising | 11.7 / 11.6MB FLAT |
| g4b_sameframe @200k (control) | flat | 10.8MB flat (unchanged) |
| g4d_lambda_sameframe @200k (control) | flat | 10.7MB flat (unchanged) |

**Tests**: classes_differential 29/29, name_resolution_differential 24/24;
closure differential probes vs CPython byte-identical (comp-frame closure
factories, nonlocal counters, two-level nesting, N=100 instance list).

**Interaction with other findings**: none (touches only the nested-def
registration path; G2's SRE tables, G5 cell cache, G8 generator registry
untouched).

**Residual (disclosed)**: `g4e_closure_hot.py` (closure-CARRYING nested
def in a hot loop) is unchanged vs pre-repair (~3.0MB/1k calls) — the
bypass path keeps wave-1 per-call `FnEntry` semantics. NOT the doc's
trigger shape (the doc's trigger, `g4c_freshframe.py`, is closure-free
and flat); fixing the closure path needs per-object closure-cell storage
(a `PyObject` field or def-time cell snapshot) — filed as follow-up.

**Verdict: PASS** (the FAILED-VERIFICATION trigger shape is flat at 3
sizes; the dedupe table is bounded; closure semantics proven unchanged by
differential).

### G6-repair — stdout capture opt-in, exit-path clone dropped (5a633ed)

**Approach.** `Interpreter.stdout_capture: bool` (new, default **false**):
- `write_out` keeps the 1MB threshold drain to fd 1 unchanged, but appends
  the drained bytes to `stdout_flushed_prefix` ONLY when `stdout_capture`
  is set.
- End-of-run: `RunOutcome.stdout` = prefix + tail when capture is on
  (byte-identical to the old behavior for embedders that opt in), else an
  **empty Vec** — no prefix copy, no exit-time `clone()` of the drained
  bytes. fd-1 output is bit-for-bit unchanged in both modes.
- Consumer audit (grep): no crate outside ap-run touches `RunOutcome`;
  ap-run-eval / ap-run-probe / ap-obj-eval / ap CLI read only
  `exit_code`; all differential suites consume the subprocess's fd 1;
  in-run readers (traceback rendering, import machinery) use
  `interp.stdout` / `write_out` directly. Nobody needed opt-in for the
  production paths — the flag exists for future embedders.

**Before/after slope** (repro `/tmp/oomverify/g6_stdout.py`):

| Probe | pre-repair | post-repair |
|---|---|---|
| g6_stdout @500k lines | 46.3MB (rising) | **11.2MB** |
| g6_stdout @2M lines | 147.6MB | **11.3MB FLAT** (64B/line slope eliminated) |

**Byte-identical stdout**:
- Small program vs CPython 3.14: stdout, stderr, rc all identical.
- 2.5MB output (60k lines, crosses the threshold drain) vs the PRE-repair
  tree (76cebda worktree build): sha256-identical fd-1 bytes
  (`473aad003960147d…`, 2,568,890 bytes both).

**Tests**: fix_pass_differential 2/2, harness_selfcheck 3/3,
import_differential 6/6, pytest tests/t14_cli 8 passed / 2 xpassed
(pre-existing xfail markers), `cargo check -p ap-run -p ap-cli` 0 errors.

**Interaction with other findings**: none (write_out drain mechanics and
all byte paths unchanged; G7's worker drain order untouched).

**Verdict: PASS** (2M-line probe flat; fd-1 bytes proven identical vs both
CPython and the pre-repair tree; the only behavior change — empty
`RunOutcome.stdout` by default — has zero consumers on this tree, verified
by grep).

### Repair commits

| Repair | SHA | Pathspecs |
|---|---|---|
| G4-repair | **44c016a** | `crates/ap-run/src/frame.rs`, `crates/ap-run/src/interp.rs` |
| G6-repair | **5a633ed** | `crates/ap-run/src/frame.rs`, `crates/ap-run/src/interp.rs` |
| this section | (this commit) | `docs/oom-fix-verification-2026-11-06.md` |

Updated bottom line: FAILED-VERIFICATION count 2 → **0** (G4 and G6 both
PASS post-repair); all other verdicts untouched.
