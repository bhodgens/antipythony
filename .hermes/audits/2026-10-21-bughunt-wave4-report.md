# Bughunt wave 4 - 2026-10-21 (commits 5f1e132..cb84991, 54 new)

Scope: 02.07 threading, uv packaging (04.01/04.02), T1 scrapy attempt
(+20 interpreter fixes), robustness-ci waves A-E (CI/test/fuzz infra),
plan/docs chores. Method: 4 auditors dispatched; ALL 4 died on provider
API timeouts with zero work (17th-20th consecutive subagent failure this
campaign), so every scope ran parent-direct with probe batteries in
worktree /tmp/ap-bughunt4. Fixes on branch `bughunt-wave4` (3 commits,
based on cb84991).

## FIXED (branch bughunt-wave4)

| Commit | Finding | Verification |
|--------|---------|--------------|
| 858f6bc | HIGH: register_iter borrow_mut race - worker parked in pull_iter holds a read borrow across the GIL-released park; main thread's unconditional borrow_mut panics 'RefCell already borrowed' -> T102 SIGABRT rc=134 ~1-in-8 runs. Fixed with try_borrow_mut retry (pull_iter's own discipline, write side). | 25/25 T102 runs rc=0; threading corpus 17/18; t10 suite 96/0 |
| ae115be | HIGH: RF-1 CRASH rc=134 - match_repr byte-sliced a String with CHARACTER indices (re.search('b','ääb').group() panicked 'not a char boundary'). HIGH: F0402-1 - repr(-1e15) rendered '-1000000000000000e+16' (exp10 counted the '-' sign; 3084 hits/350k fuzz scan). Both were FUZZER-FOUND and routed-but-unfixed by 5de2caa. Also executed the pin-flip protocol repr_fuzz documents: F0402-1 router arm deleted, pin flipped to a MATCH assertion (hard-fails on regression); F0402-2 (tie-rounding) routing kept - still unfixed. | probes byte-exact; repr_fuzz 4/4; re_fuzz 4/4; float_repr spot table green |
| 97f8ef9 | MED: RF-4 findall single-group None vs '' for non-participating groups. MED: RF-6 backrefs into open groups compiled ('((b)\1)'); CPython: 'cannot refer to an open group at position N' - parser now tracks open_groups and validates both \N and (?P=N) sites. Plus wave-1 P1 class: 3 more test suites hardcoded target/debug (packaging/import_guard/re_fuzz) - all honor CARGO_TARGET_DIR now. | seed replays byte-exact; packaging 7/7; re_differential 42/42 |

## FUZZ PROGRAM VERDICT (5de2caa's "9 real engine bugs" claim)

Claim is HONEST and the scaffold is high quality (structural routing
signatures, strict-lane env gate, pin-flip-on-fix protocol, honest
seed recording). But of RF-1..7 + F0402-1/2, FOUR were still broken at
HEAD (RF-1, RF-4, RF-6, F0402-1) - routed to "Plan 02" but no leaf had
picked them up. Now fixed except:
- F0402-2 tie-rounding (Gay's half-to-even vs Rust {:?} half-up):
  routing kept, needs a real David-Gay-style repr or digit-selection
  fix - leaf-sized, not wave-sized.
- NEW (fuzz-visible, unfiled): empty-match alternation ordering -
  'a?|b' on 'ab' -> CPython ['a','','b',''] vs impl ['a','',''].
  CPython's last-empty-match guard (bpo-32308) semantics. Documented
  in commit 97f8ef9; file for the re leaf.
- re_fuzz quick lane routes informationally (green) and only nightly
  runs strict at 5000 cases: main can accumulate routed divergences
  for 24h by design (branch-04 decision). The seed file IS the audit
  trail - this wave replayed it and found the four above.

## CI / GATE INTEGRITY (waves A+B scope)

Verified with negative controls:
- XPASS ceiling gate: at-ceiling rc0, over rc1, truncated report rc2
  (refuses to gate). Known blind spot: count-based, a +1/-1 swap is
  invisible - acceptable for burn-down, per-test ledger would close it.
- AP_REQUIRE_REFERENCE: strict mode fails (canonical message), both CI
  workflows set it - no silent-skip green path.
- Golden provenance (.meta sha256): corrupting one sha flips the suite
  red; 474 checks green restored.
- 3.14.7 pin: setup-python exact patch + provenance manifest rationale
  documented in-workflow - enforced, not aspirational.
- notices/fmt/deny gates green at HEAD.

RED AT HEAD (sibling's WIP fixes, verify after its commit):
- stdlib-provenance --check FAILS: 13 files (importlib/resources/*,
  zipfile/*, annotationlib sha) missing/stale in DEVIATIONS.toml. The
  sibling's dirty DEVIATIONS.toml adds the entries.
- importer guard message: "No module named 'a.b'" lacks CPython's
  "; 'a' is not a package" suffix - sibling's dirty importer.rs is
  rewriting this area.
- cli-docs gate: RED in my worktree only (bins unbuilt) - CI builds
  first, so CI is green. Verified zero-diff after build.

## T1 / PACKAGING (scope 2)

Clean: CLI verbs work (new/run/build/cache), uv subprocess-only (policy
honored, no linkage), purity filter matches C-08/C-10 shapes, new
vendored files byte-identical to CPython 3.14.7 (6-file sample),
scrapy gap map honest ("NOT REACHED" stated plainly).
Findings deferred to the sibling's active zone: importlib.resources
import fails at HEAD (docs describe its WIP state); 3-arg exec leaks
globals into locals; exec(code_object) rejected; class-body exec does
not populate the class; docs/t1-attempt.md blocker list stale in both
directions; `antipythany build` emits a shell shim (naming overpromises
until AoT 02.06 lands).

## THREADING (scope 1)

Byte-exact vs CPython 3.14.7: Lock/RLock/Condition/Event/Semaphore,
with-lock protocol, acquire(timeout), consistent-order 4x50 no-deadlock,
threading.local isolation, daemon/ident/active_count/enumerate.
GIL v1 discipline sound (Mutex + GilSendPub; the one race found is fixed
above). T109 stderr gap = the known traceback-frames IR-spans item.
NEW pre-existing MEDIUM (fix site contested): sorted/max/min swallow
incomparable-type TypeErrors (sorted([None,'a']) returns a list;
CPython raises) - direct comparison ops raise correctly, only the
aggregation path is wrong. builtins.rs is dirty in the sibling tree;
deferred with probe evidence.

## Gates (bughunt-wave4 @ 97f8ef9)

- cargo test --workspace: exit 0 (one transient temp-file flake in
  import_differential; 6/6 green on two reruns).
- pytest tests/: 1912 passed, 0 failed, 82 xpassed (after building
  in-tree bins; the 144-failure first run was the missing-bin artifact
  of a shared-target-only build).
- re_differential 42/42, repr_fuzz 4/4, re_fuzz 4/4 @300, packaging 7/7,
  threading corpus 17/18 (T109 known), exceptions 15/15.

## Merge note

bughunt-wave4 is based on cb84991 (main at dispatch). main has moved
(sibling committed the T1 stack); merge needs a normal (non-ff) merge -
expected clean: this wave touches sre.rs, float_repr.rs, 4 test files,
interp.rs(register_iter); the sibling's stack touches builtins/interp
regions; overlap risk is interp.rs only (different functions).
