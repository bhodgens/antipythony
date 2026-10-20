# Bughunt wave 3 - 2026-10-19 (commits 5e8a9f3..5f1e132, 16 new)

Scope: sibling's 02.04 GC-complete stack, 02.08-full p2 re engine
(1ebdf45), tuple-tracking fix (380f28e), post-re corpus state (1dbf0ff),
plus my own wave-2 commits (self-review). Method: 4 subagents dispatched;
ALL 4 died on provider API timeouts with zero work (same failure mode as
wave 2), so every scope was executed parent-direct with probe batteries.
Fixes on branch `bughunt-wave3` (worktree /tmp/ap-bughunt3), because the
main checkout is the sibling's live zone (its dirty files overlap two of
the fixed files; fast-forward blocked until its WIP lands).

## FIXED (branch bughunt-wave3, 3 commits)

| Commit | Finding | Verification |
|--------|---------|--------------|
| 2f246e9 | re engine, 6 byte-exact bugs: (1) `count=`/`maxsplit=` KWARGS silently dropped (splice guard `args.len() < 3` never true for the common 3-positional call) - `re.sub('a','X','aaa',count=2)` returned 'XXX' not 'XXa'; (2) `\Z` conflated with `$` (matched before trailing newline; fullmatch('a','a\n') True vs CPython False) - new Anchor::AbsoluteEnd; (3) group names not identifier-validated (`(?P<1x>)` compiled; CPython: "bad character in group name") - with conditional-group numeric refs kept legal; (4) huge repetition raised re.error, CPython raises OverflowError; (5) backref digits collected 3 (CPython max 2) - `\999...` error text/position wrong; (6) "unknown extension ?X at position N" off-by-one at 3 sites. | 6 probe batteries byte-exact vs CPython 3.14.7 (classes/anchors/quantifiers/groups/flags/empty-match/sub-split-count/errors); re_differential 42/42 (was 41/1) |
| a9ed8ed | e002/e014 exception regressions (self-reported in 1dbf0ff): PatternError filtered out of Exception.__subclasses__() on a wrong premise ("module-defined classes invisible"). CPython loads re at startup, so PatternError IS listed; pins regenerated from live 3.14.7 include it. | exceptions corpus slice 13/15 -> 15/15; both pins byte-match |
| b8d46a4 | Contract §9 claimed "No other divergence exists" (D1-D4) while sre.rs documents a 4M-step matching limit that aborts where CPython completes. Registered as D5. | §9 register + "D1-D5" text |

## AUDITED CLEAN (parent-verified, probes + code reading)

- GC tuple tracking (380f28e): D1-compliant probes all match - atomic
  tuples untracked, container-holding tuples tracked, post-collect
  stability, get_referents. My pre-collect probe DID diverge from CPython
  (temporarily-tracked fresh tuples), but corpus gc007 explicitly pins
  only post-collect values per D1 discipline - the divergence is in the
  UNSPECIFIED window the corpus correctly refuses to pin.
- % formatting edge battery (self-review of 2ba7d3b): dict subclass, %%
  in mapping form, %c/precision/flags with keys, unicode keys, missing
  key KeyError repr, tuple-arg TypeError - ALL byte-exact. One real gap
  found: non-dict MAPPING objects (any __getitem__ provider) raise
  "format requires a mapping" - CPython calls __getitem__. NOT fixed
  (needs a protocol call through the interpreter from ap-obj; queued).
- Crash-safety fuzz of sre.rs: 12 unwrap()/expect() sites, none
  reachable by malformed patterns (bad refs, unterminated groups, huge
  digits, 100 backrefs, bad ranges) - all raise re.error, no panics.
- xfail flips (d79508d self-review): flipped tests assert real behavior
  (dump determinism, verify diagnostics vs pins); 0 xpass remain in
  t02/t03/t04/t15.
- .pyc untrack (5f1e132): no test references the removed paths.
- tests/README.md claims spot-checked against enforcing code: .rc
  convention, _mask_root, pinned env - all accurate.

## NEW FINDINGS, NOT FIXED (queued with reasons)

| Sev | Finding | Why not fixed now |
|-----|---------|-------------------|
| MED | gc009_finalizer_raises_unraisable still fails (16/17 gc slice): impl's unraisable stderr names `__del__` where CPython names `Bad.__del__` (qualname), and rc 1 vs pinned 0. | Finalizer/unraisable report path is inside the sibling's ACTIVE 02.04 leaf (gcsupport.rs dirty in its tree). Fix there, not here. |
| MED | % formatting rejects non-dict mappings ("format requires a mapping"); CPython accepts any object with __getitem__. | ap-obj cannot call Python methods (no interpreter handle); needs a protocol hook like callable_value. Design decision for the percent-format owner. |
| LOW | re error "position" numbers still off in 2 unprobed forms; DeprecationWarning for positional count not emitted. | Position arithmetic per-site; low value, no corpus pins affected. |
| INFO | Step-limit (D5) fires on (a+)+b 'a'*22 where CPython completes in <10s. Limit may deserve a raise; now at least contract-registered. | Perf tuning belongs to the re leaf. |

## Gates (branch bughunt-wave3 @ a9ed8ed)

- cargo test --workspace: exit 0, 44 suites ok (re_differential 42/42).
- pytest tests/: 1324 passed, 0 failed, 751 xfailed, 74 xpassed.
- exceptions slice: 15/15; gc slice 16/17 (gc009, sibling's leaf).
- notices: OK (151).

## Merge note

`main` (5f1e132) is an ancestor of bughunt-wave3 - fast-forward merge,
zero conflicts expected. Blocked only by the sibling's uncommitted
builtins_modules.rs/classes.rs edits in the shared checkout. When its
WIP lands: `git merge bughunt-wave3` (or rebase), re-run the three
gates, push.

## Fixer wave (2026-10-19, "fix all issues identified using subagents")

3 fixer subagents dispatched; the re-polish one died on the provider
timeout (15th consecutive subagent failure), so all three scopes ran
parent-direct in dedicated worktrees off this branch.

FIXED (commits eee410a + 5be215d):
- re module-level positional FLAGS slot: sub/subn read flags from slot 3
  (count's slot), split needs slot 3 and sub/subn slot 4. Positional
  flags were previously misparsed as count. Byte-exact probes vs 3.14.7.
- re.error text/position battery (24 malformed patterns): 'missing
  group name' (was 'missing >'), \N{NOPE} single-quotes, (?(9)a) now
  'invalid group reference 9 at position 3' via pending-backref
  validation. Battery byte-exact.
- 4 differential suites hardcoded target/debug/ap-run-eval (wave-1 P1
  class): now honor CARGO_TARGET_DIR. This session's 2 cargo 'FAILED'
  lines were this path bug, not behavior.
- DeprecationWarning emitter for positional count/maxsplit: BUILT,
  VERIFIED correct on straight-line code, then REVERTED - the coarse
  stmt_ordinal line tracker reports wrong call-site lines once any
  function call precedes the warning site. A wrong-line warning is
  worse than none. Revisit with the IR-spans leaf.

DEFERRED (contested-zone discipline):
- gc009 unraisable report (qualname + sys.stderr swap): the sibling's
  UNCOMMITTED gcsupport.rs WIP reworks report_unraisable right now
  (Bad.__del__ qualname via e.qualname, hook None-filter, stderr
  binding resolution - exactly this fix). Would collide; it owns the
  leaf.
- %-format non-dict mappings + mapping-mode positional budget: needs
  eval_binop (interp.rs, dirty in sibling tree) and CPython's
  order-dependent budget rule ('%(k)s %s' -> not-enough-arguments while
  '%s %(k)s' works). Probed + documented here; no corpus pin or
  stdlib caller exercises it (_py_warnings uses keyed-only forms).
  Semantics for the future fixer, all probe-verified on 3.14.7:
  mapping mode iff single non-tuple arg with keys() + __getitem__;
  keyed conversions consume NO positional; positional conversions
  draw from a ONE-element budget (the mapping itself) and error
  'not enough arguments' when exhausted BEFORE rendering, except a
  positional conversion BEFORE any keyed one renders with the mapping
  as its arg (type errors then mention the mapping type).

