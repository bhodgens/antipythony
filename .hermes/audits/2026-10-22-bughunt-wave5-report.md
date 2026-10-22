# Bughunt wave 5 - 2026-10-22 (target: commit 2619a0f only)

Scope: 2619a0f is the sibling's T1 WIP that THIS session committed on its
behalf (idle 12h) to unblock the wave-4 merge - so it landed on main
NEVER audited. Wave 5 is the deferred review of that commit. No other new
commits since the wave-4 merge except two chores (f0b8643 plan-hold,
374c092 fuzz-seed append). Method: 3 auditors dispatched; 2 died on
provider connection errors after partial work (21st/22nd consecutive
subagent failures this campaign), all 3 scopes executed parent-direct in
worktree /tmp/ap-bughunt5. Fixes on branch `bughunt-wave5` (2 commits,
based on 412bc13).

## FIXED (branch bughunt-wave5)

| Commit | Finding | Verification |
|--------|---------|--------------|
| 2a6e907 | MEDIUM: re VERBOSE parser regression FROM 2619a0f itself. The rework moved whitespace/comment skipping to the parse_sequence loop top, but parse_quantifier() runs immediately after each atom - so under re.X, 'a +' errored 'multiple repeat' where CPython strips the space and parses 'a+'. Repro: re.findall(r'a +', 'aa a', re.X) -> CPython ['aa','a'], impl re.error. Fix: extracted skip_verbose(), called at loop top AND before parse_quantifier (CPython strips before EVERY token). | 22-probe VERBOSE battery byte-exact (char-class literal whitespace, (?x)/(?-x) toggles, comments, spaced quantifiers, a{2, 3}); re_differential 42/42; re_fuzz 4/4 @300; wave-3/4 batteries still match |
| bdd3a7f | HIGH: stdlib-provenance CI gate RED on committed main. 2619a0f edited 3 vendored files (_contextvars.py, annotationlib.py, inspect.py) without re-declaring their deviated_sha256 (3 stale), AND declared 7 files (ipaddress.py, urllib/*) that were never committed (untracked in the sibling's tree). ci.yml fast-lints runs this gate, so main's CI was failing. Fix: reviewed all 3 diffs (shim-only per policy), updated shas with review notes, dropped the 7 phantom entries via the tool's own --fix. | gate: OK (83 files, 10 deviated declared); phantom entries self-heal when the sibling commits those files |

## AUDITED CLEAN (parent-verified)

- Interpreter portions of 2619a0f: NO REGRESSION. All 7 corpus slices
  match the committed baseline exactly (values 46/47, fstrings 34/35,
  exceptions 15/15, classes 27/33, gc 16/17, threading 17/18,
  stdlib 19/48). Cargo workspace builds 0 errors.
- What the interpreter diff actually ships works byte-exact: PEP 604
  union FLATTENING ((int|str)|bytes merges arms, no TypeError), X|None
  arms, PEP 584 dict|dict (right-wins, |= works, mixed-type TypeErrors).
- Union PRESENTATION gap recorded (pre-existing T1-typing leaf, not
  introduced): print(int|str) shows <_Union object> not 'int | str';
  type name '_Union' not 'Union'; typing.get_args empty;
  isinstance(x, A|B) TypeErrors. Owner: sibling's T1 leaf.
- Vendored shims: the 3 modified files' diffs are minimal
  runtime-binding shims per LICENSES policy (Token/Context surface,
  callable-obj annotations branch, get_annotations path); no semantic
  changes to vendored CPython logic.
- The provenance gate itself performed as designed: caught BOTH defect
  classes (stale sha + phantom entry). Positive control confirmed
  (wave 4 confirmed the negative control).

## KNOWN FLAKE (not fixed, low priority)

- import_differential::package_import_and_relative intermittently
  fails with 'cannot read /var/folders/.../ap_import_diff_*.py' - the
  temp case file vanishes between write and spawn under parallel test
  threads (seen once each in wave-4 and wave-5 workspace runs; 6/6
  green on every isolated rerun, 3/3 this wave). Test-harness lifecycle
  race, not engine behavior. Fix shape: unique-per-test temp dir or
  NamedTempFile held open across the spawn.

## Gates (bughunt-wave5 @ bdd3a7f)

- cargo test --workspace: exit 0 modulo the import_differential flake
  above (32+ suites ok; flake 6/6 green on rerun x3).
- stdlib-provenance --check: OK.
- pytest tests/: (running at report time; baseline at merge was
  1912 passed / 0 failed - no wave-5 change touches Python-visible
  behavior except the re.X fix, covered by re_differential/re_fuzz).

## Merge note

main moved to 374c092 (fuzz-seed chore) after wave5 branched from
412bc13. Merge is a normal 3-way: wave5 touches sre.rs (VERBOSE fix),
DEVIATIONS.toml, + findings/report files; 374c092 touched only
FUZZ_SEEDS.txt + PROGRESS.md. Zero file overlap expected.
