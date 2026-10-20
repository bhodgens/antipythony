# Branch 04 - Differential Fuzzing

Goal: property-based differential tests for the two highest-yield surfaces
the review names (finding #9): the hand-written regex backtracker and the
repr paths. Any random input where CPython and ap-run disagree byte-wise is
a shipped bug, and no hand-probe list finds those. Nightly, not PR-blocking.

## Leaves

### 04.01 - proptest differential fuzz for re
Files: `crates/ap-run/tests/re_fuzz.rs` (new), ap-run dev-dependencies
(proptest - license check via cargo-deny before adding; proptest is MIT).
- Strategy: generate pattern strings from a grammar biased to the engine's
  constructs (classes, quantifiers greedy/lazy, groups incl. named,
  backrefs \1-\9, alternation, lookaround, flags subset M/S/I/X), plus
  subject strings (small alphabet + unicode pockets).
- For each pair: run `ap-run-eval`-style probe (or the engine in-process
  via ap-run's re surface) AND `python3.14 -c` via the 00/02 shared
  reference helper; compare stdout bytes (match/span/groups/sub result)
  and error texts.
- Shrink to minimal repro; failing seeds append to
  `tests/corpus/fuzz-re/` as permanent corpus cases (corpus only grows).
- Respect the documented divergences (step-limit error message, bytes
  patterns latin-1 gap): whitelist by exact bytes, never by "close".
- Keep runtime bounded: proptest cases configurable via env
  (AP_FUZZ_CASES, default 1000 local / nightly higher).
Gate: `cargo test -p ap-run --test re_fuzz` green over seed-stable run;
one planted divergence (e.g. mutate sre.rs quantifier laziness temporarily
in a scratch check) trips a red - verify the fuzzer can fail.

### 04.02 - proptest differential fuzz for repr paths
Files: `crates/ap-obj/tests/repr_fuzz.rs` (new) or ap-run equivalent,
whichever exposes the eval seam cheaper.
- Float repr: bit-random doubles (finite + inf/nan + subnormals +
  extremes), compare `repr()`/`str()` bytes vs reference (float_repr.rs is
  the hand-written formatter - the classic bug farm).
- Int repr: bigint-sized random ints (num-bigint range), decimal/hex/oct/bin
  forms; int->str conversion paths.
- Same shape as 04.01: shrink, promote failures to corpus, whitelist only
  documented divergences (range() bigint clamp per AGENTS/PROGRESS notes).
Gate: green; planted formatter bug trips red.

### 04.03 - Nightly fuzz workflow
File: `.github/workflows/nightly-fuzz.yml` (schedule cron only; master
contract note: workflow in the review sketch). Build release bins (speed
matters for fuzz), set AP_FUZZ_CASES high, run 04/01 + 04/02 suites, upload
failing-seed artifacts, notify nothing (results land in CI summary).
Gate: workflow dispatches manually once green; schedule block present.

## Dispatch notes

Hard dependency: 00/02 (shared reference discovery + AP_REQUIRE_REFERENCE).
04.01 and 04.02 parallel-safe (different crates). Sequence AFTER the re
engine's own leaf wave settles (it is fresh in-tree; fuzzing it mid-edit
wastes runs).
