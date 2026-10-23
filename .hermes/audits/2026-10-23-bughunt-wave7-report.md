# Bughunt wave 7 - 2026-10-22/23 (full-repo, 10 auditors, HEAD c51d78d)

Scope: full-repo sweep - every crate, the gate ecosystem, and crash-fuzz.
Method: 10 auditors dispatched in parallel; 9 died or partially completed on
provider errors, but ALL 10 left findings files (the incremental-write
discipline from waves 1-5 held). Every scope was executed or completed by
its auditor; parent verified the highest-impact findings and spot-checked
the rest. Findings at /tmp/ap-audit-w7-*/findings.json (10 files).

## THE HEADLINE: 10 CONFIRMED EXECUTOR SEMANTICS BUGS (scope: interp.rs)

The executor auditor (survivor, 4.7h) found 10 real bugs, all parent-
verified. These are CORE Python semantics broken in the tier-0 executor:

HIGH:
1. for/else + while/else: break does NOT suppress the else clause (always
   runs). CPython: else runs only on completion without break.
2. Default arguments evaluate at CALL time, not DEF time
   (x=10; def f(a=x); x=20; f() -> 20 vs CPython 10).
3. Comprehension lambda defaults capture the final loop value
   ([lambda x=i: x ...] -> [2,2,2] vs [0,1,2]); the bare-lambda late-
   binding form instead crashes NameError - two bugs, one site.
4. raise X from Y does not set __cause__.
5. Implicit __context__ chaining not readable on caught exceptions
   (raise-site chaining logic exists; the attribute surface is missing).

MEDIUM:
6. return in finally does not supersede the in-flight exception.
7. StopIteration.value attribute missing.
8. Generator send/throw/close control flow incorrect (premature StopIteration).
9. Function annotations evaluated at def time (CPython 3.14 stores lazily
   per PEP 649) - part of the T1 typing surface.

## OTHER CONFIRMED BUGS (parent-verified)

### ap-obj / builtins (objmodel + nativemods scopes)
- pow(2,-3,5) raises ValueError; CPython computes the modular inverse (2).
- int() parsing: rejects 1_000_000 underscores, whitespace-before-base,
  0x/0o/0b prefixes for the matching base.
- Binop TypeError messages name 'object' instead of the operand class
  ('object and int' vs "'L2' and 'int'").
- range membership with huge/bigint bounds: TypeError ('needs an int in
  v0') - CPython O(1) arithmetic check.
- json.loads rejects NaN/Infinity (CPython accepts -> nan/inf).
- math.floor/ceil on huge floats clamp to i64::MAX (CPython: exact bigint).
- itertools.count ignores start/step; accumulate ignores initial=.
- functools.lru_cache cache_info() returns None (CPython: CacheInfo).
- functools partial(partial(f,1),3)(4) - nested partial fails.
- os.path.splitext('..hidden') -> ('.','.hidden'); CPython ('..hidden','').
- io.StringIO/BytesIO getvalue() after close() doesn't raise ValueError.
- time.monotonic() returns near-zero (not increasing); strftime('%Z')
  returns literal '%Z'.
- typing.get_type_hints() returns {}; inspect.signature() returns [].
- collections.defaultdict + most_common AttributeError after nested lambda
  default.
- sys.version_info repr is opaque object (CPython: named tuple).

### crash-fuzz (crashfuzz scope)
- CRITICAL: [1] * (2**62) -> Rust 'capacity overflow' panic (rc 134);
  CPython raises MemoryError. File: alloc raw_vec via list-multiply path.
- CRITICAL: re.compile('(?iabc)def(?i)') -> unreachable-code panic in
  sre.rs:949 parse_inline_flags; CPython raises re.error.
- ap-ir-dump + ap-parse-check: no crashes on any of ~114 inputs (clean).
- Known shlex.split stack overflow + deep recursion: not re-reported.

## GATE ECOSYSTEM VERDICT (gates scope) - sound, with notes
- PIN ROT: 5 corpus families re-generated vs live 3.14.7 - ALL match.
  No pin rot.
- t08 cross-mode tests: DO anchor against CPython reference bytes (not
  self-confirming) - refuted the self-confirming hypothesis.
- CI wiring: fast-lints covers traceability/notices/provenance; rust job
  covers fmt/clippy(info)/build/test; attribution + nightly-fuzz active.
- Skips: all env-conditional or documented-optional; none hide failures.
- xfail: 74-ish t08 envelope is the bulk (known); no NEW stale xfails
  outside t08 found beyond those already in the ledger.

## FRONTEND (ap-parse/ap-ir scope)
- HIGH: parse diagnostics NOT byte-exact - 15/18 battery cases mismatch
  (message text + line/col format vs C-10 shape).
- HIGH: missing diagnostics for return/yield outside function, duplicate
  args, nonlocal-without-binding (ap-parse-check silent; CPython raises).
- MEDIUM: ap-ir-dump shows AST-like nodes, not lowered IR (may be
  intentional presentation choice - flag for the dump-format owner).
- MEDIUM: verify pass possible false-negative on class-body scope
  isolation (probe: class C: x=1; def f(): print(x)).
- CLEAN: unbound-local must-analysis core (nested func + module cases
  byte-exact); div-by-zero correctly raised at RUNTIME; ap-parse/ap-ir
  cargo suites + pytest t01-t04 all green.

## CORRECTIONS TO EARLIER WAVES
- docs files in c51d78d (02-10-scheduler-design.md, t1-regex-risk-map.md):
  CONFIRMED never committed (staged only; sibling's pathspec miss) - the
  docs auditor's CRITICAL finding stands (ADDENDUM 3).
- selectors.Selector finding from wave-6 REFUTED (fails identically in
  CPython - not a public name).

## TALLY (confirmed bugs by area, this wave)
- executor: 10 (for/else x2, def-time defaults, comprehension scoping x2,
  __cause__, __context__ surface, finally-return, StopIteration.value,
  generator send/throw, annotation eval)
- nativemods: 15 (json NaN, math.floor clamp, itertools.count/start,
  accumulate initial, lru_cache cache_info, nested partial, splitext,
  io getvalue-after-close, time.monotonic, strftime %Z, get_type_hints,
  Union repr, inspect.signature, defaultdict most_common, sys.version_info)
- objmodel: 4 (pow modular inverse, int parse underscores/whitespace/
  prefix, binop TypeError operand names, range membership bigint)
- crashfuzz: 2 CRITICAL (list-multiply capacity panic, re inline-flag
  unreachable panic)
- frontend: 6 (parse diagnostics byte-exactness, 4 missing SyntaxError
  classes, IR dump AST-like presentation)
- importgc: 2 (gc.collect count divergence - un-pin-able, sys.path
  shadowing facets incl. ANTPYTHANY_PATH)
- tooling: 1 (timeout kill orphans grandchildren)
- license/compliance (from wave 6, still open): third-party attribution

TOTAL confirmed open: ~40 distinct bugs + the license/provenance
compliance items + the deferred design items (IR-spans, F0402-2,
empty-match ordering, sorted TypeError path).

## Merge note

bughunt-wave7 branch: report + findings JSONs only (docs, no code - all
code findings live in the sibling's contested zone). FF-merge into main
safe; no code fixes to conflict.
