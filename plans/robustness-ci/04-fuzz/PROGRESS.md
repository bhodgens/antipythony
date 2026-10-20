# Branch 04 PROGRESS

## 04.01 IMPLEMENTED NOT COMMITTED (2026-10-20, leaf agent sa-2)

### What landed
- **`crates/ap-run/tests/re_fuzz.rs` (new, ~940 lines)** — proptest
  pattern/subject differential fuzz for the re engine vs live CPython 3.14
  via the 00.02 shared seam (`common::reference_or_skip()`), same probe +
  `target/debug/ap-run-eval` byte-compare shape as re_differential.rs.
  - Grammar biased to engine constructs: literals/escapes/classes,
    greedy/lazy/possessive quantifiers `{m,n}`, groups incl. named +
    digit-name rejects, backrefs `\1` and `(?P=name)` (incl. the
    open-group arm), alternation, lookarounds (fixed + variable-width
    arms), anchors, inline flags `(?i)(?s)(?m)(?x)` global AND scoped
    forms incl. negation `(?-i:` (surface no hand probe covers),
    conditional groups, atomic groups, invalid-repeat arms (error-text
    channel). Unicode pockets (`ä ß ١ 中`) on both pattern and subject.
  - Case count from `AP_FUZZ_CASES` (default **200**, quick lane;
    nightly 04.03 raises). `AP_FUZZ_STRICT=1` flips the lane: every
    unwhitelisted divergence fails and proptest shrinks to a minimal
    repro. `AP_FUZZ_TIMEOUT_MS` (default 5000) caps each subprocess
    wall (poll + pipe-reaper threads — never wedges the suite;
    reference-side timeouts are skipped + counted, honest to the
    documented "CPython hangs" divergence). `AP_FUZZ_EVAL_BIN` lets
    nightly point the suite at its own release build.
  - Probe exercises search/match/fullmatch/findall/finditer/split/sub/
    subn/`subn(count=1)`/expand with span/groups/groupdict/lastindex/
    lastgroup/repr + compile-error text; compares stdout BYTES + exit
    codes; reference stderr asserted byte-empty (probe is 3.14-
    warning-free).
  - **Whitelist = EXACT bytes, two documented divergences only**
    (docs/conformance.md): (1) `ap: matching step limit exceeded` —
    our stdout containing that exact string where CPython answered is
    skipped + counted; (2) bytes-pattern latin-1 gap — **str patterns
    only** chosen; bytes surface excluded from the fuzz by generation,
    not by fuzzy comparison.
  - Failure output: every divergence is printed (`DIVERGENCE [family]`
    + ref/our bytes + panic message) AND appended to
    `plans/robustness-ci/04-fuzz/FUZZ_SEEDS.txt`, deduped and capped at
    8 lines/family/run (the file stays a representative replay ledger,
    not a flood). proptest's own regression files disabled
    (`FileFailurePersistence::Off` + fork off; the `Off` variant exists
    in 1.12 even with the `fork` feature unified on via sibling 04.02).
  - `harness_smoke_case`: deterministic byte-exact anchor through the
    same machinery. `strict_lane_can_fail_scratch_proof`: planted
    divergence goes red under `AP_FUZZ_STRICT=1` (gate 3, permanent
    strict-only form). The one-shot scratch mutated-bytes test was run
    and removed per the brief.
  - Cargo.toml: `[dev-dependencies] proptest = { version = "1",
    default-features = false, features = ["std"] }` — ap-run's first dev
    section; MIT OR Apache-2.0 (deny.toml allow-list covers it;
    cargo-deny binary absent locally, license verified via
    `cargo metadata`). No lib feature unification (dev-deps are
    dev-only; verified `cargo tree -e normal` clean).
- Lane result (default 200, debug): **green** — 4/4 tests; 785/1500 of a
  1500-case debug run were routed findings, honest-printed, not masked.
  `re_differential` still 42/42 green. rustfmt --check clean.

### ROUTED FINDINGS (engine bugs — Plan 02 owns sre.rs; DO NOT fix here)
Verified minimal repros, byte-diff vs live `python3.14` 3.14.7:

- **RF-1 PANIC (crash, rc134): `Match.group()`/`repr(m)` slice the byte
  string with char indices.** `sre.rs:2073` `match_repr` does
  `mb.text[start..end]` where spans are char positions. Repro:
  `re.search('b', 'ääb').group()` → panic
  "byte index 2 is not a char boundary; it is inside 'ä'". Any match
  ending after a multibyte char crashes on group()/repr. Highest-severity
  finding: crashes the interpreter thread, hits every unicode subject.
  Seed family `PANIC-rc134`.
- **RF-2 `sub`/`subn` ignore keyword `count=` (all 4 entry points —
  module-level, pattern-method, compiled, uncompiled; positional
  `count` works).** Repro: `re.sub('a','-','aaa',count=1)` → ours
  `'---'` (3 subs) vs CPython `'-aa'` (1). The kwargs splice in
  `builtins_modules.rs:1585` is gated on `args.len() < 3` but the
  splice result is not what the Pattern-method path reads / compiled
  dispatch drops it; family `subn1` (18/785 of the routed findings at
  1500 cases — the single most-hit family).
- **RF-3 lookbehind/zero-width at pos 0: `(?<=…)` at the start of the
  subject is evaluated one position late.** Repro:
  `re.search('(?<=a)','a1').span()` → ours `(0,0)` vs `(1,1)`;
  `re.findall('(?<!a)','ab')` → ours `['']` vs `['','']`. Anchored
  `match()` misses pos-0 lookbehind hits (`(?<!\w)` family `search`
  lines). (Lookahead at 0 is fine.)
- **RF-4 `findall` returns `None` for non-participating groups; CPython
  returns `''`.** Repro: `re.findall('(a)?','b')` → ours `[None,None]`
  vs `['','']`. Same for split's captured-group list. Family `findall`.
- **RF-5 `lastindex` overcounts on repeated alternation groups.**
  Repro: `re.match('((a)|b)*','ab').lastindex` → ours `2` vs `1`
  (CPython reports the LAST group that matched a non-empty capture;
  our engine counts the reset/unset participation).
- **RF-6 open-group backref accepted.** Repro: `re.compile('((b)\\1)')`
  → ours compiles; CPython: `PatternError cannot refer to an open
  group at position 4`. Also `(?P<1>x)`: CPython rejects digit group
  names ("bad character in group name"), ours compiles. Family
  `COMPILE_MISSED`.
- **RF-7 compile error text/position divergences on some repeat shapes.**
  `re.compile('\\Z{2}')` → ours "at position 4" vs CPython "position 2";
  `(a){2}{3}` / `1*+{1,2}`: CPython raises "multiple repeat", ours
  accepts (possessive+repeat chain); `\11` backref positions off by one.
  Family `COMPILE_TEXT`.

Fixing these is 02.08 product work with its own review; the two
documented divergences in docs/conformance.md do NOT cover any of
RF-1..RF-7 (they are bugs, not sanctioned divergence). The strict lane
(nightly 04.03) will go red on these until Plan 02 lands fixes — that
is the designed outcome of finding #9 ("zero fuzz" → real bugs found
on the first 200-case run).

### Seeds
`FUZZ_SEEDS.txt`: 35 replay lines after the final default-lane runs
(cap 8/family/run), families PANIC-rc134 / subn1 / search / findall /
COMPILE_MISSED / COMPILE_TEXT / fullmatch. Replay = paste the Debug-escaped
pattern/subject into the probe files; the `DIVERGENCE [family]` print
carries ref-vs-our bytes verbatim.

### Notes for the orchestrator
- The 04.01 lane is intentionally non-blocking (quick lane = print + seed,
  per branch-04 "Nightly, not PR-blocking" + Plan-02 engine ownership).
  The brief's "AP_FUZZ_CASES=1500 green or report REAL divergences"
  branch executed: 1500 cases ran (debug AND release), lane stayed green,
  and the real divergences are RF-1..RF-7 above — that IS the required
  output, not a skip.
- Ownership held: this leaf touched only `crates/ap-run/tests/re_fuzz.rs`,
  `crates/ap-run/Cargo.toml` [dev-dependencies], `Cargo.lock` (proptest
  add via cargo), `plans/robustness-ci/04-fuzz/PROGRESS.md` +
  `FUZZ_SEEDS.txt`. sre.rs NOT touched; no commits made.
- `xtask notices --check` drifts while Cargo.lock carries the proptest
  add (regenerating THIRD_PARTY_NOTICES produced a 7000-line churn diff
  because the tool now re-wraps existing entries too — too invasive for
  this leaf to leave staged; the orchestrator's commit step should run
  `cargo run -p xtask -- notices` and squash, or accept the reflow).
  proptest license = "MIT OR Apache-2.0" — deny.toml allow-list covers it.

## 04.03 IMPLEMENTED NOT COMMITTED (2026-10-20, leaf agent 04.03)

### What landed
- **`.github/workflows/nightly-fuzz.yml` (new)** — nightly differential-fuzz
  workflow. `on: schedule cron '0 3 * * *'` + `workflow_dispatch` (manual
  proof trigger). Single job `fuzz` (ubuntu-latest, timeout-minutes 120):
  checkout → dtolnay 1.94.0 → setup-python 3.14 → `Export reference path`
  (`AP_CONFORM_REFERENCE_PYTHON`, mirrors ci.yml:79-82) → rust-cache →
  `cargo build --release -p ap-cli --bin ap-run-eval` →
  DEFAULT-lane re_fuzz + repr_fuzz at `AP_FUZZ_CASES=5000` (job gate;
  informational-green BY DESIGN while Plan 02 owns RF-1..7 / F0402-1/2) →
  STRICT-lane steps (`AP_FUZZ_STRICT=1`) with `continue-on-error: true`,
  each named/commented "red-until-Plan-02-fixes-by-design" with the
  self-destruct condition in the header comment (when sre.rs/float_repr.rs
  are fixed the pin test `routed_findings_pins_currently_reproduce` flips
  red on purpose → delete router + pins + these two steps) →
  upload-artifact (4 lane logs + FUZZ_SEEDS.txt + FUZZ_SEEDS_0402.txt) →
  job-summary step greping the suites' own tallies into $GITHUB_STEP_SUMMARY.
- Env names verified against source (not guesses):
  `re_fuzz.rs:80 env_usize("AP_FUZZ_CASES", 200)`, `re_fuzz.rs:85
  std::env::var("AP_FUZZ_STRICT")`, `re_fuzz.rs:176
  std::env::var("AP_FUZZ_EVAL_BIN")` (fallback `target/debug/ap-run-eval`;
  the workflow sets the release path explicitly — re_fuzz.rs:172-175
  comment names this leaf), `repr_fuzz.rs:77/125` (AP_FUZZ_CASES /
  AP_FUZZ_STRICT; in-process suite, no eval bin). Reference seam reads
  `AP_CONFORM_REFERENCE_PYTHON` (tests/common/mod.rs:56) and
  `AP_REQUIRE_REFERENCE=1` set workflow-level (finding #3).
- Summary-step greps validated against REAL proof-log shapes:
  `^re_fuzz summary:` line, `DIVERGENCE \[family\]` | sort | uniq -c
  tally (simulated: 85 PANIC-rc134 / 23 subn1 / 23 search / 18
  COMPILE_MISSED / 3 COMPILE_TEXT / 1 findall at 300 cases), and the
  repr `ROUTED FINDINGS` print — which repeats per case with CUMULATIVE
  counters, so the step takes `tail -1` for the final tally. `|| true`
  wraps the greps inside `$( ... )` (pipeline status would otherwise
  bypass `|| echo`).

### Gates executed
- YAML parses: `python3.14 -c "import yaml; ..."` OK (pyyaml 6.0.3);
  cron/timeout/step list asserted programmatically.
- `bash -n` on every `run:` block: all OK.
- Local reduced-scale proof (cannot run a remote action; proved the exact
  command sequence on this host, Apple Silicon, rust 1.94-era toolchain,
  reference python3.14 = /opt/homebrew/bin/python3.14 3.14.7):
  - `cargo build --release -p ap-cli --bin ap-run-eval` — 10.9s warm.
  - `AP_FUZZ_CASES=300 AP_FUZZ_EVAL_BIN=target/release/ap-run-eval
    cargo test -p ap-run --test re_fuzz --release -- --nocapture` —
    GREEN 4/4, 12.65s, summary line `cases=301 ok=148 routed=153`,
    153 DIVERGENCE prints + seed lines appended (working-as-designed
    ledger churn, see gaps).
  - same scale `-p ap-obj --test repr_fuzz` — GREEN 7/7, 6.87s, routed
    F0402-1 layout=24 F0402-2 tie=9 (final tally).
  - STRICT smoke (20 cases): re_fuzz FAILED with shrunk minimal input
    `(?>\b)/a` [subn1]; repr_fuzz FAILED 4 tests (pins fire) — proving
    the continue-on-error informational steps are genuinely red today,
    exactly as designed.
- Nightly estimate (honest scale-up): fuzz itself is subprocess-spawn
  dominated and near-linear — 5000/300 ≈ 16.7× → re ~3.5 min + repr
  ~2 min (host; assume GH runner ≈2× slower → ~10-12 min default lane);
  strict lane short-circuits at first divergence + shrinks (~1-2 min).
  The long pole is the release profile build of the test targets/crates
  (rust-cache warm: full job ~15-25 min; cold cache: release build of
  the dep graph adds ~15-30 min → worst case ~45-60 min). Comfortably
  inside timeout-minutes: 120.

### Gaps / notes for the orchestrator
- Remote action run NOT executable from here (subagent, no repo push/gh
  dispatch permission): proved the LOCAL equivalent of every step's
  command; the action-level wiring (cron firing, artifact upload,
  $GITHUB_STEP_SUMMARY) is standard actions already mirrored from ci.yml.
  First real `workflow_dispatch` run should confirm.
- `git status` shows the leaf's own proof runs appended 45 new lines to
  `plans/robustness-ci/04-fuzz/FUZZ_SEEDS.txt` (deduped replay ledger —
  the suite is designed to grow it; commit or `git checkout` per
  orchestrator preference). repr_fuzz writes no ledger file.
- Ownership held: touched ONLY `.github/workflows/nightly-fuzz.yml` (new)
  + this PROGRESS.md append. ci.yml untouched. No commits.
