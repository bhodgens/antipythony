# Leaf 02RESID-A session record — Plan 03 post-gate native divergences (2026-11-22)

Worktree `~/.git/worktrees/leaf-02resid-a`, branch `tier1-02resid-a` off main
@ 1cd873c. CARGO_TARGET_DIR=$PWD/target-02resid-a throughout.

## Status: 4/4 cases byte-exact vs CPython 3.14.7, corpus pins green

## Case dispositions

1. **V016 (parser surrogate escapes)** — ALREADY CLOSED ON TRUNK, not by this
   leaf. The parser shim landed pre-branch (d3d7be4 ap-parse surrogate-literal
   WTF-8 re-decode, ac1a182 ap-obj WTF-8 code-point surfaces, f0c629d ap-run
   surrogate-aware codec engine) and the corpus pin
   `values/expected/V016-encode-decode-errors.*` is already in-tree and green.
   This leaf verified the case byte-exact vs python3.14.7 (stdout+stderr+rc
   diff empty) and closed it verification-only. The V016-root-cause.md note
   ("parser layer, NOT fixed") is STALE — the fix landed after 03.07 wrote it.

2. **t1_007 round(float, ndigits)** — FIXED (708861a). Root cause in
   `round_half_even` (builtins.rs): `int_digits = (exp+1).max(0)` CLAMPED the
   exponent at 0 AND `format_decimal` LEFT-padded the digit string to the
   point position — round(3712500.0, 6) became 37125.0 (3 orders lost);
   round(0.000123, 7) became 0.123 (unclamping alone fixed that half).
   Reworked: keep = exp+1+ndigits (no clamp), rebuild via shortest-repr
   scientific string (identical binary64 parse in Rust/CPython), trailing-zero
   trim compensated into the scale. Also: `decimal_round_half_even`'s
   point<0 arm fell through with empty keep and rounded 6.3e-05/ndigits=3 up
   to 0.001 — against the exact 40+-digit expansion the 5-unit threshold is
   unreachable there, so the arm now always returns zero. 415-case
   differential sweep (randomized exps/ndigits incl. ties 8.835/2.675,
   negative ndigits, subnormals) all byte-exact.

3. **c018 vars() on __slots__** — FIXED (e7503ac). CPython semantics: slot
   members are NEVER in __dict__; vars() raises TypeError
   "vars() argument must have __dict__ attribute" for dict-less classes;
   dir() still lists slot names. Implementation: slot writes on
   has-instance-dict classes park in a new SLOT_STORAGE runtime_state registry
   (instance-addr keyed, Weak pins, retired at run teardown); no-dict classes
   keep the legacy attrs layout; builtin_vars returns the LIVE attrs handle
   for dict-backed instances (pre-existing snapshot copy broke
   vars(o)[k]=v mutation visibility — en-route fix); slot delete dispatches
   before the property "no deleter" arm. 13-probe slots sweep + corpus pin
   byte-exact. Pre-existing NOT-slots gaps documented, untouched: class dict
   lacks __doc__/__firstlineno__/__static_attributes__, module qualname in
   <class '...'> repr.

4. **c032 metaclass resolution** — FIXED (767d7b8). Two root causes:
   (a) the explicit metaclass kwarg returned UNCONDITIONALLY — CPython's
   type.__new__ picks the MOST DERIVED among {explicit} ∪ {type(base)}; a
   non-strict-subclass chain conflict raises the pinned TypeError;
   (b) a metaclass whose MRO lacks `type` (plain class) was accepted as a
   metaclass payload — CPython CALLS it with (name, bases, ns) as a plain
   callable ("NotAMeta() takes no arguments" / function metaclasses bind
   their return). 10-case metaclass sweep + corpus pin byte-exact.

## Gates (measured this session, this worktree)

- Corpus pins: all four cases diff-empty vs /opt/homebrew/bin/python3.14.7
  (stdout+stderr+rc).
- ap-conform slices (--tier 0): values 48/48 (V016 green; slice grew 47->48),
  classes 30/33 (was 28/33 on the stale trunk binary — c018+c032 now green;
  c002/c030/c031 pre-existing, verified byte-identical failures on trunk).
- Full native corpus sweep: 322 passed / 17 failed / 1 timeout (was 305/32/3
  on the stale trunk binary; my fail set is a strict subset — zero
  regressions).
- cargo test -p ap-run --release: 18 suites ok / 0 failed (baseline held).
  GOTCHA re-confirmed: fix_pass_differential needs
  `cargo build -p ap-cli --bin ap-run-eval` in the SAME target dir first
  (fresh worktree has no debug/ap-run-eval).
- cargo test -p ap-parse --release: 16+7 passed / 0 failed (parser not
  touched this leaf; suite green anyway).
- Full workspace suite NOT run (sibling lanes own their suites this window).

## Files touched

- crates/ap-run/src/builtins.rs (round/format_decimal, dir slots arm, vars)
- crates/ap-run/src/classes.rs (slot storage/reads/deletes, metaclass
  resolution, instance_dict_handle_pub)
- crates/ap-run/src/interp.rs (object.__getattribute__ __dict__ slots guard)
- crates/ap-run/src/runtime_state.rs (SLOT_STORAGE_T + pins + retire rows)

## Findings

- Stale-trunk-binary gotcha: /Users/caimlas/git/antipythony/target/release is
  MONTHS old (predates the V016 fixes, codecs_cjk e0xx lineno waves) — using
  it as a "trunk baseline" over-reports failures (32+3 vs the true current
  trunk state). Comparisons should rebuild or diff against git, not the stale
  bin.
- V016-root-cause.md should be updated to CLOSED (shim landed d3d7be4
  post-03.07) — left untouched (out of this leaf's write scope per the file
  plan; noted here for the orchestrator).
- c030/c031 (metaclass __call__ chains: `super(): no arguments` inside
  Meta.__call__, Bad.__init_subclass__ kwargs) and c002 (class-body scoping
  NameError) remain the classes-slice reds — documented partial in 02.03b
  PROGRESS, NOT addressed by this leaf.
