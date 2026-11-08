# T-08 — tier-1 transparency (gates implementation leaf 02.05 ap-codegen-native tier-1)

Authorship: written from the compat contract + spec pack ONLY (`docs/compat-contract.md`
C-31 tier/mode invisibility + C-29 exit-code and stream invariants + C-24/C-27 channel
contract + C-28 `sys.exit` semantics + C-19..C-22 finalization-timing rules as they bind
corpus construction; `docs/spec/01-values-and-objects.md`, `docs/spec/02-control-flow.md`,
`docs/spec/03-classes.md`; `plans/antipythony/master.md`;
`plans/antipythony/02-native-backend/orchestrator.md` 02.05 Expose/Gate), per
`plans/antipythony/tests/README.md`. The author has not read any implementation source
under `crates/`.

## Purpose

Leaf 02.05 adds a tier-1 compiler to the native backend: CLIF lowering of hot functions,
inline caches, guarded specialization, on-stack replacement (OSR), and deopt that must
restore the **tier-0 exact mid-state**. The correctness rule under test is C-31 / the leaf's
own words: **tier-1 is invisible except by timing** — any observable difference is a bug.
T-08 therefore tests **transparency, not speed**:

1. Every conforming program, run under forced tier-0, adaptive, and AoT-binary modes,
   produces **byte-identical stdout, byte-identical stderr, and the same exit code**.
2. Programs designed to *force* tier-1 activity — inline-cache feedback, OSR into a hot
   loop, deopt on a type change — still produce exactly the bytes CPython 3.14 produces.
   A deopt that restores a wrong mid-state corrupts loop-carried values, exception state,
   or generator state, and that corruption is directly visible in the pinned output.
3. The speed gate (">=1.5x geomean over tier-0 on the arithmetic/string benchmark slice")
   is expressed as a **measured-report stub**, never an assertion about another
   implementation's timing, because timing is not a conformance channel (C-24).

## Testability contract under test (from leaf 02.05's Expose/Gate)

- Expose: CLIF lowering, inline caches, guarded specialization, OSR, deopt-to-tier-0
  exact mid-state.
- Gate (verbatim): "adaptive-mode full corpus green; >=1.5x geomean over tier-0 on the
  arithmetic/string benchmark slice. Conformance suite runs both forced tier-0 and
  adaptive modes."
- C-31 (normative): "Adaptive execution (02.05 inline caches, OSR, deopt) and AoT-vs-JIT
  differences MUST NOT change stdout bytes, stderr bytes, or exit code of any conforming
  program. Forced tier-0, adaptive, and AoT binary modes MUST agree on every corpus
  case." Verify clause (verbatim): "a 1M-iteration loop with OSR pressure MUST print the
  reference sequence exactly once each (no duplicated or dropped side effects across an
  OSR/deopt boundary)."
- C-29/C-28 ride along: the agreement includes the exit-code channel — uncaught
  exception → 1, `sys.exit(n)` → n & 0xFF, unraisable-at-exit → rc unchanged. A mode
  flag that changes any of these is a C-31 violation.
- C-24/C-27: the compared channels are exactly stdout bytes / stderr bytes / exit code,
  captured separately, byte-exact, no normalization. Timing, addresses, and `id()` are
  explicitly NOT compared — T-08 probes must therefore contain no clock/`id()`/random
  reads (enforced by `test_no_timing_or_identity_apis`), so a transparency test can never
  flap for a non-conformance reason.
- C-33 (corpus monotonicity): the cross-mode suite **globs `tests/corpus/**/*.py` at run
  time** — whatever exists at gate time is in scope, and the corpus only grows.
- CLI seam / resolution order: `APX_IMPL` env (whitespace-split) → `antipythony` on PATH
  → `target/{debug,release}/antipythony`, then `run`. Per-mode commands default to
  `<prefix> --tier 0`, `<prefix> --tier adaptive`, `<prefix> --mode binary` and are fully
  overridable with `APX_MODE_TIER0_CMD` / `APX_MODE_ADAPTIVE_CMD` / `APX_MODE_BINARY_CMD`.
  Reference: `AP_CONFORM_REFERENCE_PYTHON`, default `/opt/homebrew/bin/python3.14`.
  Flag vocabulary is not pinned by any document this author may read — see GAP G-1.

## Cross-mode byte-identity rule (the core assertion)

For every script S in the glob `tests/corpus/**/*.py`, run S three ways:

```
antipythony run --tier 0     S    → (o0, e0, r0)
antipythony run --tier adaptive S  → (oa, ea, ra)
antipythony run --mode binary  S    → (ob, eb, rb)
```

Require `o0 == oa == ob` byte-for-byte, `e0 == ea == eb` byte-for-byte, `r0 == ra == rb`
— C-31 first two sentences; the exit-code equality is also C-29/C-28 because the modes
must each keep uncaught-exception/SIGINT/`sys.exit`/unraisable codes identical. When the
corpus directory also carries pinned reference triples (`expected/<stem>.{stdout,stderr,exit}`
from values/tier1 slices, or `.{out,err,rc}` from classes/exceptions), tier-0 must match
the CPython reference bytes too, closing the three-way loop to the semantics reference.
The three runs are separate stream captures under a scrubbed environment (`PATH`, `HOME`,
`LANG` only), per S-06-2 / C-27.

## Specialization-probe corpus

`tests/corpus/tier1-transparency/` — 15 probe programs + `bench/` speed slice (4
programs), each with `expected/` triples recorded from **real** runs of
`/opt/homebrew/bin/python3.14` (3.14.7), run twice per program to prove determinism.
Loop counts are 10k–50k by design (tier-0 mode runs them slowly on purpose; the budget
is enforced by `test_iteration_budget_bound` — see G-5 for why C-31's "1M-iteration"
verify text is capped here). Probes print only deterministic aggregates (sums, lengths,
masked slices, type names), never addresses.

### Probe table — program → deopt/OSR/IC mechanism stressed → clauses cited

| Program | Mechanism stressed | Cites |
| --- | --- | --- |
| t1_001_hot_int_loop_osr.py | OSR: plain `for` over `range` crosses the hotness threshold mid-loop; int `+`/`*` IC specialization carries `total`/`prod` across the tier boundary | S-01-1, S-02-16, S-02-17, C-31 |
| t1_002_mixed_type_callsite_deopt.py | call-site IC: `double()` specializes on int (30k calls), is forced to **deopt** by 200 str calls, then re-specializes on int; exactness of all three phases pinned | S-01-1, S-01-14, S-02-11, C-31 |
| t1_003_attr_ic_class_switch.py | attribute-IC polymorphism: one `obj.x` read/write site alternates `__dict__` class / `__slots__` class every iteration — guard failure + deopt path per switch | S-03-09, S-03-10, C-31 |
| t1_004_osr_side_effects_once.py | OSR mid-state restore: side effects every 2500 iterations; a **duplicated or dropped** append/accumulation at the OSR/deopt boundary changes `len(marks)` and sampled marks — C-31 Verify verbatim ("no duplicated or dropped side effects") | S-02-16, S-02-17, S-01-19, C-31 |
| t1_005_generator_send_types.py | generator resume across type feedback: one generator resumed 20k times, `send` value flips int/str/None — yield-point frame state must survive deopt at every resume; `StopIteration.value` tuple pinned | S-02-20, S-02-21, S-02-22, C-31 |
| t1_006_hot_string_build.py | string-IC specialization in a hot loop: `+=` concat + f-string + join mixed; lengths and 24-char prefixes pin every iteration ran exactly once | S-01-14, S-01-16, S-02-17, C-31 |
| t1_007_promotion_then_typeerror.py | int×float specialization (S-01-7 promotion) at 30k calls, then a str input forces guard failure into the **reference TypeError message byte-exactly** on the post-deopt error path | S-01-6, S-01-7, S-01-8, S-02-29, S-02-30, C-31 |
| t1_008_dict_hot_ic.py | dict get/setitem IC + version-guard invalidation via periodic negate/re-read of the first 10 keys — insertion order (S-01-21) invariant across every tier transition | S-01-21, S-01-23, C-31 |
| t1_009_list_push_pop_shape.py | list append/pop/clear/extend drive repeated list-internal representation changes under a hot method-IC site; starred unpack pins per-iteration counts | S-01-19, S-02-19, C-31 |
| t1_010_shortcircuit_identity.py | truthiness/bool-IC churn: falsy↔truthy and user-`__bool__` operands alternate; `is` on the sentinel pins operand identity (not `id()`) after any deopt | S-01-27, S-01-28, S-01-29, S-01-32, C-31 |
| t1_011_hot_instantiation.py | type-call IC: 20k `Point(...)` calls through one site + instance `__setattr__` specialization; allocation pressure crosses IC thresholds mid-run | S-03-05, S-03-06, S-03-10, C-31 |
| t1_012_deopt_midstate.py | **exception-driven deopt to tier-0 exact mid-state**: raise/catch/finally five times inside a 40k loop — loop counter, local accumulator, module global, and finally-counts must all resume exactly | S-02-1, S-02-3, S-02-30, S-02-33, C-31 |
| t1_013_megamorphic_binop.py | megamorphic `+`: one binop site cycled through six type pairs (int, float, str, list, tuple, int+float) every 200 iterations — repeated guard failures, IC megamorphic-stub path | S-01-1, S-01-7, S-01-14, S-01-19, S-01-20, S-03-25, C-31 |
| t1_014_exceptions_in_hot_loop.py | unwind-to-handler inside tier-1: ZeroDivisionError + LookupError raised/caught 4.7k times in a 20k loop; caught/kind exact sums pin that no iteration's handler state was skipped or doubled | S-02-29, S-02-30, S-02-31, C-31 |
| t1_015_nested_break_else.py | control-flow traces across OSR: nested while/for with break, continue, and both `else` clauses — a mis-restored control stack at the tier boundary rewrites the deterministic (o, tag) trace | S-02-15, S-02-16, S-02-17, S-02-18, C-31 |
| t1_016_deep_call_chain_500.py | 500-deep chained Python calls (`f(n) = 1 + f(n-1)`) — debug-build stack-overflow gate: each level adds one interpreter frame; passes only if the runtime survives depth 500 with def-time binding intact | S-02-7, S-02-10, C-31 |

| bench/ program | Slice role | Cites |
| --- | --- | --- |
| bench01_int_arith.py | arithmetic (hot int +,-,//,%) | S-01-1, S-01-3, S-01-4, S-02-17, C-31 |
| bench02_float_arith.py | arithmetic (IEEE float chain) | S-01-6, S-01-7, S-01-8, C-31 |
| bench03_str_ops.py | string (f-string/join/upper, 20k parts) | S-01-14, S-01-15, S-01-16, C-31 |
| bench04_bigint_mul.py | arithmetic (growing bignum mul) | S-01-1, S-01-2, C-31 |

Bench programs are **also transparency cases**: their `expected/` triples participate in
the cross-mode suite and the probe differential; only their *wall time* is measured, and
only by the stub below.

## Speed-gate handling decision

The 02.05 gate says ">=1.5x geomean over tier-0 on the arithmetic/string benchmark
slice." Timing is explicitly **not** a conformance channel (C-24: "timing... explicitly
non-observable for conformance purposes"), so the speed gate CANNOT be a byte-identity
assertion and can never be made green by fixing a semantic bug. Handling:

- `test_speed_gate_geomean_over_tier0` is a **measured-not-asserted harness stub**,
  `xfail(strict=False)`. It runs each of the 4 bench programs WARMUP=1 + REPEATS=3 times
  under tier-0 and adaptive, computes a conservative per-program speedup estimate
  `min(tier-0 times) / max(adaptive times)` (best honest tier-0 divided by worst honest
  adaptive — a lower bound), takes the geometric mean over the slice, **writes a JSON
  report** (`APX_T08_REPORT`, default `tests/t08_tier1/bench_report.json`), and asserts
  `geomean >= 1.5`.
- Before 02.05 lands the body raises inside the XFAIL envelope (no impl / flags). After
  it lands, the report is machine-readable evidence the orchestrator reads alongside the
  pass; the assertion is the gate. Floor-tolerance and environment questions live in
  GAP G-3/G-4 below.

## Per-test citations (suite `tests/t08_tier1/test_t08_tier1.py`)

### Half A — fixture re-verification (green today)

| Test | Assertion | Cites |
| --- | --- | --- |
| `test_probe_slice_quota` | ≥12 probes, ≥4 bench programs, corpus tree ≥100 cases | task contract; C-33 |
| `test_expected_triple_exists` ×19 | `.stdout`/`.stderr`/`.exit` present for every probe+bench | C-24/C-27 channel triple |
| `test_header_cites_existing_clauses` ×19 | line 1 is `# Cites:`; every `S-nn-m` id exists in specs 01–03; every `C-nn` id is a live contract clause; C-31 cited by every case | tests/README.md corpus rules; C-32 (ids immutable → validate against live doc) |
| `test_stresses_header_present` ×19 | each program annotates the tier-1 mechanism it forces | 02.05 Expose list |
| `test_reference_reproduces_expected_bytes` ×19 | re-run under python3.14 == pinned bytes on all three channels | C-01 reference; C-27 |
| `test_expected_streams_are_clean` ×19 | rc 0, empty stderr, newline-terminated stdout for every case | C-29/C-30 (clean exit), spec-02 path-text avoidance |
| `test_no_nondeterministic_addresses_leak` ×19 | no `0x…` token in any expected file | C-24, S-06-17 #4 |
| `test_no_timing_or_identity_apis` ×19 | no `time`/`id()`/`random`/refcount reads in any case body | C-24 (non-channels); C-19/D3 (refcount prohibited); C-23 (no timing assertions) |
| `test_iteration_budget_bound` ×19 | all `range()` literals ≤ 200_000 | G-5 budget decision |
| `test_probe_table_sync` | this table names exactly the probe files; mechanism coverage floors (osr / deopt / inline cache / generator / megamorphic / slots) | C-31 Verify (OSR/deopt must be exercised) |

### Half B — antipythony-side gates (xfail(strict=False) until 02.05 lands)

| Test | Assertion | Cites |
| --- | --- | --- |
| `test_cross_mode_byte_identity` ×(whole-corpus glob) | tier-0 ≡ adaptive ≡ binary on stdout/stderr/rc; when pinned triples exist, tier-0 ≡ reference | C-31 (all three sentences), C-29, C-28, C-24/C-27; C-33 (glob at run time) |
| `test_cross_mode_suite_runs_over_grown_corpus` | suite sanity: all corpus slices present at gate time + one adaptive run exits 0 | C-33; 02.05 gate ("adaptive-mode... green") |
| `test_probe_matches_reference_bytes` ×15×3 modes | every probe, every mode, byte-equal to its pinned python3.14 triple | C-31; per-probe S-cites above |
| `test_osr_side_effects_exactly_once_adaptive` | explicit C-31 Verify spot: no duplicated/dropped side effects across the OSR boundary | C-31 Verify verbatim |
| `test_adaptive_mode_full_corpus_green` | every corpus case under adaptive == reference bytes/rc | 02.05 Gate verbatim half 1 |
| `test_speed_gate_geomean_over_tier0` | geomean(conservative speedup) ≥ 1.5 on bench slice + JSON report emitted | 02.05 Gate verbatim half 2 (measured, not assumed; C-24 timing carve-out) |

## CONTRACT-GAPS

- **G-1 — tier/mode CLI vocabulary is not pinned by any readable document.** The leaf
  text says "conformance suite runs both forced tier-0 and adaptive modes" and the
  task brief names `--tier 0` / `--tier adaptive` / `--mode binary`; 00.04's ap-conform
  CLI (the only documented harness) exposes no tier/mode flags in its section as read
  by this author. The suite assumes the three defaults and makes each mode a full
  command-line override (`APX_MODE_*_CMD`), so the gate stays runnable if the landed
  seam names the flags differently. The leaf document (or a spec-06 PR) should pin the
  CLI once 02.05's Expose is final.
- **G-2 — "byte identity across modes" vs "AoT binary mode" is two seams in one.**
  `--mode binary` in the suite's default expansion still runs via `antipythony run`
  with a flag; the true 02.06 artifact path is `build` then execute the binary. Until
  02.06 lands, `APX_MODE_BINARY_CMD="/path/to/antipythony build-and-run"` (or the T-09
  binary suite) carries the artifact interpretation. C-31's native-vs-wasm half is
  T-12's scope, explicitly excluded here (wasm has no tier-1 in v1 — divergence D4).
- **G-3 — timing gate is inherently environment-dependent; floor-tolerance decision.**
  A wall-clock geomean on shared CI hardware has ≥±10-20% run-to-run noise. Decision
  recorded here (per "measured not assumed"): the gate threshold is the contract text,
  **1.5x exactly, no tolerance band baked into the assertion**, because the assertion
  runs against the *same host's* tier-0 baseline in the same process session with
  warmup + 3 repeats + conservative estimator — the estimator biases the measured
  speedup DOWNWARD, so environmental noise makes a true 1.5x implementation more likely
  to read low, not high. The orchestrator reviewing a 1.42x "failure" reads the JSON
  report: if per-program ratios are all ≥1.4 and one outlier dragged the geomean, that
  is a re-run candidate, not a gate waiver. No corpus case may assert timing (C-24);
  this stub is the ONLY place timing is touched, and it touches it as an internal
  ratio, never as a conformance byte.
- **G-4 — "arithmetic/string benchmark slice" has no pinned program list in any doc
  this author may read.** Decision: the 4 bench programs here (2 arithmetic-int, 1
  float, 1 bigint + 1 string) ARE the slice for gate purposes until the leaf names its
  own; corpus-only-grows (C-33) means later additions join the geomean, never replace
  these. If 02.05 ships a different named slice, amend THIS document via PR and keep
  the old programs.
- **G-5 — probe loop budget vs C-31's "1M-iteration loop" verify text.** C-31's Verify
  paragraph names a 1M-iteration OSR-pressure loop; the task rule for this corpus is
  10k–100k because every case runs under forced tier-0 three-plus times per CI pass and
  a 1M tier-0 interpretation would dominate runtime (and tier-0 here is the *slow* mode
  by design). Compromise recorded: OSR is exercised by 50k loops crossing the hotness
  threshold (thresholds are impl-internal; 50k exceeds any plausible per-branch counter
  for CLIF tier promotion), plus the explicit `test_osr_side_effects_exactly_once_adaptive`
  spot. If 02.05's real threshold turns out to be above 50k on any loop shape, bump the
  constants in t1_001/t1_004 (corpus grows, never weakens — C-33 direction is UP).
- **G-6 — deopt mid-state has no direct observation channel.** Byte-identity is an
  indirect proof: a corrupted mid-state must eventually print wrong. A deopt bug in
  state that is never printed is invisible to T-08 by construction (and is legal to be
  invisible per C-24 — but NOT legal to be observably wrong). If the impl exposes
  instrumentation (deopt counters via env var / `--stats`), an added differential
  "deopt actually fired during these probes" test would upgrade this from
  transparency-only to transparency+activation coverage. Not assumed here — no doc
  promises it. Filing back as a spec-06/leaf-doc suggestion, not a contract claim.
- **G-7 — reference-build drift (shared caveat).** Expected bytes recorded on
  `/opt/homebrew/bin/python3.14` = 3.14.7; C-01 pins 3.14.0 and marks non-pinned-patch
  bytes provisional (OPEN item in 00-foundation PROGRESS.md). Probes deliberately avoid
  3.14.0→3.14.7-sensitive surfaces (no repr corner cases, no traceback text in expected
  files — every probe swallows its exceptions in-script and prints only `Type: message`).
- **G-8 — C-29 SIGINT (exit 130) not probed here.** Signal-driven KeyboardInterrupt
  during tier-1 execution is a real deopt surface (interrupted machine code must unwind
  through tier-0 frames) but requires injecting a signal at a chosen time — outside
  the deterministic-file corpus model of ap-conform. Owned by T-09 (exit-code clause
  family) for the un-signal'd cases; the mid-tier1 SIGINT case needs an integration
  test once the CLI has a stable run verb. Recorded, not silently dropped.

## Run command

```
/opt/homebrew/bin/python3.14 -m pytest tests/t08_tier1 -q
```

Overrides: `APX_IMPL` (impl command prefix), `APX_MODE_TIER0_CMD`,
`APX_MODE_ADAPTIVE_CMD`, `APX_MODE_BINARY_CMD` (full per-mode command lines),
`APX_T08_TIMEOUT` (per-run seconds, default 120), `APX_T08_SPEED_REPEATS` (default 3),
`APX_T08_REPORT` (benchmark JSON path), `AP_CONFORM_REFERENCE_PYTHON` (reference).

## xfail policy

- All Half-B tests are `xfail(strict=False)` — the implementation is in flight in a
  sibling session, so XPASS on early cases is **expected and reported, not a failure**;
  a strict xfail would fail the suite precisely when the leaf starts landing, inverting
  the signal. After 02.05 completes: any XPASS is a passing gate test; any FAIL is a
  gate failure for 02.05 (fix the implementation, never the test — master.md protocol,
  C-33).
- Half-A tests must stay green. A Half-A break means the recorded reference bytes
  drifted (G-7) or the corpus was edited (C-33 review event), never an antipythony bug.
- A probe failure under exactly one mode (green under forced tier-0, red under adaptive)
  IS the designed signature of a tier-1 bug: wrong mid-state after OSR/deopt. The
  failure text points at the mode and the mechanism table above names what that probe
  stresses.

## Protocol notes

- Tests commit BEFORE 02.05 completes (README §Protocol 3); the gate-time runner is an
  agent that wrote neither these tests nor the tier-1 code (§Protocol 4).
- Spec/leaf contradictions found: G-1 (unpinned CLI seam), G-4 (unpinned benchmark
  slice), G-5 (verify-text loop size vs corpus runtime budget) are leaf-document/spec
  PR candidates; G-6/G-8 are coverage notes for future leaves. None were resolved by
  silently picking semantics — each carries an explicit, overridable decision above.
