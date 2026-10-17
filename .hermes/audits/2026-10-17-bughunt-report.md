# Bughunt 2026-10-17 - consolidated report

Wave anchor: `4f3d033..4bf9f77` (all 38 commits, first to HEAD).
Method: 10 read-only auditors (one per commit batch, each in its own detached
git worktree) + parent pre-reads + parent source-verification of every HIGH.
Fixes applied parent-direct this session (the request authorized "fix issues
as you find them").

## Baseline at clean HEAD (before fixes)

- `cargo test --workspace`: RED. 24 suites ok, 1 suite red:
  `ap-run/tests/stdlib_modules_differential.rs` -> 19/19 panic
  `spawn ap-run-eval: Os { code: 2, kind: NotFound }`.
- `pytest tests/`: 4 failed, 1265 passed, 165 skipped, 594 xfailed, 90 xpassed.
- `cargo run -p xtask -- notices --check`: RED (NOTICES DRIFT).

## Baseline at clean HEAD (after this wave's fixes)

- `cargo test --workspace`: green (see gate log).
- `pytest tests/`: 1271 passed, 5 skipped, 751 xfailed, 91 xpassed, 0 failed.
- `notices --check`: OK (150 deps).

## FIXED (parent-verified, committed this wave)

| # | Sev | Finding | Fix | Verified |
|---|-----|---------|-----|----------|
| P1 | HIGH | `stdlib_modules_differential.rs` hardcoded `target/release/ap-run-eval`. `cargo test` builds debug, so all 19 tests panic NotFound on a tree without a manual release build; when a stale release binary exists they silently diff against NON-HEAD code. | Moved test to `crates/ap-cli/tests/` (owns the bin) + `env!("CARGO_BIN_EXE_ap-run-eval")`. | 19 panic -> 16 pass / 3 real fails |
| P2 | HIGH | 4 corpus stderr pins embed the absolute checkout path (`/Users/caimlas/...`). CPython 3.14 resolves script paths to absolute in tracebacks/warnings, so the pins fail in every worktree/CI other than the original. | `_mask_root()` regex normalizes any absolute prefix before `tests/corpus/...` to `<ROOT>/...`; line numbers, caret columns, message text untouched (C-33 mask-not-weaken). Applied to all 7 stderr-diffing suites. | 4 pytest fails -> 0; mask validated vs 4 pins + negative control |
| P3 | HIGH | `s005_io_streams` golden is locale-dependent. Commit f0a4c22 claimed the reference prints `utf-8 utf-8` "under all local locales" - FALSE: `io.text_encoding(None)` returns `'locale'` outside UTF-8 mode. Root cause: CPython's UTF-8-mode auto-detection rewrites `LC_CTYPE` in its own env block, so a Python parent leaks it to children and flips their locale. The runners passed the ambient env, not the S-06-15 scrub. | `_pinned_env()` in every corpus runner: PATH/HOME only + `LANG=C` (PEP 540 enables UTF-8 mode for the C locale). ap-conform `run_with_timeout` now `env_clear()` + PATH/HOME/LANG=C for BOTH sides (S-06-15 harness contract). | golden matches under scrub; 113 pin tests green under `PYTHONUTF8`/scrub |
| P4 | MEDIUM | `find_reference_interpreter` fell back to any `python3` with only a spawn check, no version check. A host without python3.14 silently used e.g. 3.12 as the "3.14 reference" and still printed green. | `is_python_314()` parses `--version` (stdout+stderr) and requires `Python 3.14`. Same guard added to the 3 Rust differential test files. | probe with python3.14 hidden now fails closed |
| P5 | LOW | `ap-conform` declared unused dep `shell-words`. | Removed from Cargo.toml. | grep: zero uses |
| N1 | HIGH | `ap-conform` rejected `--tier` (`error: unexpected argument`), but the 02.03 gate + T-06/T-07/T-08 all invoke `--tier 0`. The tier-0 conformance gate could not run as specified. | Added `--tier <0|adaptive>` to ap-conform (forwarded to impl only, never the reference; bad value -> exit 2). `ap-run-eval` now parses `--tier` and rejects non-0 loudly instead of silently running tier-0. | `--tier 0` PASS; `--tier 7` exit 2; `--tier adaptive` fails loudly |
| N2 | HIGH | `notices --check` gate was RED at HEAD: `THIRD_PARTY_NOTICES` stale vs Cargo.lock (162 lock packages vs 151 entries). The attribution gate AGENTS.md treats as a compliance requirement was not green. | Regenerated notices (150 deps after shell-words removal). | `notices --check: OK` |

## REFUTED auditor claims (parent source-verification)

- Auditor-2 F1 "corpus glob misses 100 of 148 cases": FALSE. values/classes/
  exceptions have no subdirectories, so `*.py` matches all (47/33/15).
- Auditor-4 F5 "nondeterministic HashMap iteration in the byte-compared IR
  dump": FALSE. ap-ir uses only BTreeMap/BTreeSet (verify.rs:535, tests.rs:219).
- Auditor-5 F3 "x012_keyboard_interrupt pin has wrong exit code": FALSE. Pin
  says 130, real CPython run gives 130.
- Auditor-5 F6 "t16_hybrid collects zero tests": FALSE. 33 tests collected.
- Auditor-8 "int.bit_length / to_bytes missing": OUT OF SCOPE. Not in the v1
  spec (grep of spec 01/05 finds no bit_length/to_bytes). Feature gap, not a bug.
- Auditor-9 "`is` interning divergences (257 is 257, "abc" is "abc")": REFUTED
  by spec S-01-31 - object caching/interning is UNSPECIFIED, corpus MUST NOT
  assert `is` except on None/sentinels.
- Auditor-3 "--verify never wired into ap-ir-dump": FALSE. `ap-ir-dump
  [--tree|--golden|--verify]` exists and `ap-verify-dump` runs (rc 0).

## REAL defects found but NOT fixed this wave (deferred, with reason)

| Sev | Finding | Why deferred |
|-----|---------|--------------|
| HIGH | Traceback frame leak: a CAUGHT exception's frames reappear in the NEXT traceback. Repro: `try: f() except: pass` then a later error shows `line 2, in f` + `line 7, in <module>` instead of just the real frame. `tb_frames` (frame.rs:257) accumulates on the interpreter and is never cleared when an exception is handled. | File is in the sibling's uncommitted 02.04a WIP (interp.rs/frame.rs). Contested zone - defer, don't force. |
| MED | 3 stdlib differential failures (stat/os_path/linecache) at the working tree. | Caused by the sibling's uncommitted importer WIP; pass at clean HEAD. Their build failure, their fix. |
| MED | `"%d" % (t,)` (percent-format) and complex-number arithmetic unimplemented. | Tracked feature gaps (02.08-full: "re engine, complex numbers, class-method surface"). Not regressions. |
| LOW | Traceback caret line (`~~^~~~~~~`) missing in impl output. | Known Plan-02 gap; line NUMBERS are correct (verified). |

## Addendum: auditor 7 recovery (post-report)

Auditor 7 (f-string battery + BUG-1/walrus fix review) failed the output
schema after 1 retry, but its findings survived on disk
(/tmp/ap-audit-7/audit_results.json). Verdict: both fixes CORRECT
(f-string debug_text lowering in ap-ir/expr.rs, walrus bind_definite +
comprehension leak in ap-ir/verify.rs); 35-case battery pins verified;
no conformance issues. Its one actionable finding (MEDIUM doc drift) is
fixed in wip-02.04a commit 7238cc5: docs/pon-study-lowering.md still
flagged the ruff "parts-not-elements" gotcha as a possible live bug;
0992ee9 resolved it as not-a-bug.

## Addendum: auditor 1 recovery (post-report)

Auditor 1 (scaffold commits 4f3d033/d01d477/24c97ed) also failed the
output schema; findings survived on disk (/tmp/ap-audit-1/audit_results.json).
Parent-verified dispositions:
- HIGH CONFIRMED: `notices --check` was declared a "CI gate" (xtask
  docstring, master.md:45, commit messages) but NO CI config existed -
  the gate only ran when someone remembered. This wave proved the
  failure mode (notices stale at HEAD). Fixed: .github/workflows/
  attribution.yml (commit 4f99399).
- MEDIUM CONFIRMED: LICENSES/ lacked the MIT/Apache pon entries AGENTS.md
  requires before the first pon-derived commit. Pre-staged from the
  license-pinned clone (commit 630acea).
- LOW REFUTED: empty `[[licenses.exceptions]]` in deny.toml is
  intentional scaffolding ("Empty for now" comment) - same class the
  auditor itself marked clean elsewhere.

## Addendum: auditor 10 recovery (post-report)

Auditor 10 (02.03/02.08-lite/license commits + sibling WIP review) failed
the output schema; findings survived at /tmp/ap-audit-10/bughunt-audit-findings.json.
Parent-verified dispositions:
- HIGH REFUTED: `parent_of()` catch-all `_ => Some("Exception")` in
  exc_hier.rs. Probes at HEAD match CPython: `class MyError(BaseException)`
  is NOT caught by `except Exception`, and isinstance/issubclass give
  False. The except path routes user classes through the real MRO
  (legacy_exc_is_subclass is LEGACY_KNOWN-guarded, interp.rs:1017); the
  table path only sees builtin names.
- LOW CONFIRMED + upgraded: LICENSES/README-vendored-stdlib.md promised a
  PSF entry in THIRD_PARTY_NOTICES "via the xtask static-notices list" -
  the mechanism never existed. Implemented: xtask static_entries() adds
  'CPython vendored stdlib v3.14 / PSF-2.0' when vendored_stdlib/ exists
  (commit 1936698). Verified both states green (150/151 entries).
- INFO/clean: MRO C3 linearization correct; MRO error messages match
  CPython wording; sibling-WIP disclosures match this report's ledger.

## Addendum: auditor 6 recovery (post-report)

Auditor 6 (pon license grant 0122e90, IR verify pass f0a4c22, notices
regen 5552cf9) failed the output schema; findings survived at
/tmp/ap-audit-6/audit_result.json. Parent-verified dispositions:
- HIGH DUPLICATE (already fixed): LICENSES/ lacked MIT/Apache pon texts.
  Same finding as auditor 1; fixed pre-emptively in commit 630acea.
- HIGH REFUTED: "'CPython byte-matched' claim rests on a single test."
  False: tests/t04_ir_verify/ is a 32-probe systematic suite whose
  fixtures (expected_t04.json/.txt) are GENERATED from live CPython
  3.14.7 (gen_expectations.py) and the suite compares class+message
  fields for every flagged probe. Suite green: 15 passed, 29 xpassed.
- MEDIUM x2 clean (auditor's own verdicts): div/floor-div/mod folding
  correctly refuses zero divisors (runtime ZeroDivisionError preserved);
  1MB string-repetition fold cap is intentional conservatism.
- INFO x2 clean: notices determinism (BTreeMap + --check) confirmed;
  attribution headers correctly pending first pon-derived commit.

## Addendum: auditor 5 recovery (post-report)

Auditor 5 (T-07/T-09/T-11 corpora, spec amendments A-1..A-6, T-14/T-15/T-16
suites, commits 2057bf8/3f1bb33/71193a6/ebf5956) failed the output schema;
findings survived at /tmp/ap-audit-5/audit_results.json. All 10 findings are
INFO/clean — no fixes needed. Parent spot-verified three against live
CPython 3.14.7:
- A-1 CONFIRMED: `1.0/0.0` → `ZeroDivisionError: division by zero` (not
  "float division"). Amendment correct.
- A-4 CONFIRMED: `except ((KeyError, ValueError),):` → `TypeError: catching
  classes that do not inherit from BaseException is not allowed`. Amendment
  correct (nested tuples rejected).
- A-6 CONFIRMED: `weakref.ref(set())` succeeds; `weakref.ref(list())` →
  `TypeError: cannot create weak reference to 'list' object`. Amendment
  correct (set/frozenset are weakref-able).
- T-07 gc corpus: 3-run byte-identical, no flake (auditor verified; matches
  this wave's pytest green run).
- T-14/T-15/T-16 xfail envelopes: GREEN NOW assertions present, not empty
  placeholders. Matches the 91-xpass count in this wave's gate run.
- The recovered JSON's 10 findings are all INFO/clean. This auditor's one
  HIGH finding (--tier gate seam, report row N1) was delivered and fixed
  earlier in the wave (commit 06a086f).

## Addendum: consolidated batch dispositions (final)

The 10-auditor batch closed at 4h19m. All findings processed; two new
fixes landed from the consolidated payloads:
- CRITICAL CONFIRMED + FIXED (auditor 9): deep recursion SIGABRTs the
  process at ~150 Python frames (stack overflow) before the
  recursion_limit=1000 guard (frame.rs:387) can fire. Repro: depth-300
  script rc=134 at clean HEAD. Fix: ap-run-eval runs the module on a
  256MB-stack thread so RecursionError raises as a Python exception
  like CPython (commit 5fafcf0). Root fix (smaller interpreter frames)
  is an executor follow-up.
- HIGH CONFIRMED + FIXED (auditor 2): C-02's promised traceability CI
  check ("every cited divergence id exists", compat-contract.md:72)
  was never implemented - per-suite lints only checked header
  PRESENCE. New tests/corpus_traceability/ resolves every cited id
  against contract + spec-pack ids; negative control (bogus S-99-42)
  flips it red (commit 885b526).
Other consolidated findings dispositioned without code change:
- Timeout-vs-FAIL (auditor 2 CRITICAL): REFUTED - CaseOutcome::Timeout
  fails the gate (passed()==false, exit 1); the label is diagnostic.
- Negative-control claim (auditor 2 HIGH): fixed earlier (26822c6).
- e012 .exit missing (auditor 3 CRITICAL): REFUTED - the expected-file
  convention is .rc; e012 has .out/.err/.rc.
- "Corpus has no pinned triples" (auditor 2 INFO): REFUTED -
  tests/corpus/*/expected/ holds pinned .out/.err/.rc triples.
- xfail strict=False (auditor 3 HIGH): real; same item as the 91-xpass
  ledger entry - user triage.
- Float-repr exponent boundaries (auditor 8 MEDIUM): verified clean at
  HEAD - probe matched CPython on 1e16/1e17/1e-4/1.2e-5/1e23/1.5e-323.
  The probe's only diffs were complex-number results ((-2)**0.5 ->
  complex in CPython, nan in impl) - the tracked 02.08-full complex gap.
- powf boundaries (auditor 9 MEDIUM): float cases matched CPython;
  negative-base fractional exponent is the same complex gap.
- Traceback line numbers (auditor 9 MEDIUM): verified correct earlier
  (caret line missing is the known LOW gap).
- ruff 0.0.4 pin (auditor 3 MEDIUM): MSRV-pinned by design (commit
  24c97ed message); documented, not a defect.

Final gate state (main @ 885b526, clean worktree): cargo exit 0;
pytest 1272 passed / 0 failed / 91 xpassed; notices OK (150 deps).

## Post-wave git state (disclosure)

A sibling agent kept working in the main checkout during and after this
wave. It committed its 02.04a stack (06df71b, 83a7125, f4c409f) on a
detached HEAD branched from 4bf9f77 - WITHOUT this wave's 6 commits,
which sit on `main` (d43e734, ahead 10). The two stacks touch ZERO
common files (verified: git diff --name-only 4bf9f77..main vs
..f4c409f, empty intersection), so a merge is conflict-free by
construction. The sibling's stack is preserved on branch `wip-02.04a`
(+ my doc fix 7238cc5 on top). The main checkout is currently on that
detached sibling line with further uncommitted WIP (importer.rs,
vendored_stdlib/collections.py, functools.py) - left untouched.
Reconcile: merge wip-02.04a into main (or rebase) once the sibling's
WIP lands; then re-run the three gates on the merged tree. NOTE: after
the merge, `notices --check` will report drift on purpose - the new
xtask static_entries() adds the PSF entry once vendored_stdlib/ exists;
run `cargo run -p xtask -- notices` and commit the result as part of
the merge.

## Disclosure ledger (what was NOT verified / NOT touched)

1. SIBLING WIP: the working tree carried another agent's uncommitted 02.04a
   work (importer.rs, vendored_stdlib/, edits to builtins/classes/exc_hier/
   frame/interp/lib/methods.rs) written 11:04-11:06, minutes before this
   session. Auditor 10 reviewed it read-only. This wave did NOT edit or commit
   any of those files. The 3 stdlib differential failures and the traceback
   frame leak live in that WIP.
2. 91 xpassed (was 90): stale `xfail(strict=False)` markers not flipped to
   pass. Pre-existing (same count at baseline). Not a correctness bug, but the
   markers understate what now works. Not triaged this wave.
3. 5 skipped tests not investigated (bin-not-found skips).
4. Auditor findings on ap-parse ruff-wrapper fidelity and ap-obj repr/hash
   exactness (auditors 3, 7, 8) were reviewed but not all independently
   re-probed by the parent; the ones the parent did probe (f-string debug
   forms, bigint exactness, int->float conversion) MATCHED CPython at HEAD.
5. Gate runs: cargo + pytest + notices were run on a clean detached worktree
   (/tmp/ap-fixcheck) with only this wave's fixes applied, to separate my
   fixes from the sibling WIP. The working-tree cargo run shows the 3 sibling
   failures; the clean-HEAD run is the authoritative verdict for this wave.
6. No push performed (user decides push timing).
