# Leaf 02.05c — Typed regions + guards + cold twin (the tier-1 core)

Status: LANDED on `tier1-0205c` (base 814b249 = main). Branch delivers the
full pon pipeline: feedback snapshot → infer → region finder →
`OptimizingPlan {regions, guards, fast_path, cold_twin}` → typed CLIF
emission → synchronous tier-up at the call boundary.

## Repr decision (the leaf's one open design call — DECIDED)

**Unboxed repr = raw `i64` CLIF register, admitted only within ap-obj's smi
domain** (`Int::Smi`, `[-2^59, 2^59)`, crates/ap-obj/src/int.rs SMI_MIN/
SMI_MAX). Float/str reprs are explicitly deferred (repr-agnostic pipeline;
02.05h bench wave decides if float pays for itself).

Guards are **VALUE-level** (type + range), re-checked on every region load
and per typed op — nothing caches across executions. Consequences:

- Unlike pon, this pipeline consumes **no version counters**. The 02.05b
  type_version/surgery_version counters remain for the future IC-consuming
  codegen (attr/global caches); the feedback snapshot's role here is a
  **hotness gate** (`infer::has_int_hotness`: ≥1 binop cell MONO(IntI64,
  IntI64)), not a type source. Recorded deliberate divergence from pon
  (pon types temps from feedback because its guards are version-based).
- Admission lattice (infer.rs): `SmiConst` (in-domain literal, static),
  `SmiLocal` (LoadLocal result — sound through the runtime `h_slot_smi_v`
  sentinel guard), `SmiBin` (typed-op result, operands unboxable,
  exactness runtime-checked), `Bool` (relational Compare/Truthy over
  unboxable operands → 0/1 i64 boxed as `Value::Bool`).

## What landed

- **infer.rs** — per-temp repr lattice over the cfg; block-local walk;
  site count via the SHARED `clif::specializable` predicate (Risk-5: one
  definition of "owns a feedback slot"; drift impossible by construction,
  cross-checked at compile time against SlotPlan::len — assert-density
  analogue of pon's `debug_assert_suspend_numbering`).
- **region.rs** — maximal single-entry unboxable-region finder: contiguous
  eligible stretches, single-entry enforced by shrinking from the right
  (predecessor sets); per-block disqualifier reports (diagnostics, never
  silent). `typed_binop` family = add/sub/mul/div/floordiv/mod.
- **optimizing.rs** — `OptimizingPlan {regions, guards, fast_path,
  cold_twin}`; **fast-entry soundness rule**: only regions rooted at
  block 0 are fast-pathed (a region whose entry is a branch target would
  execute unconditionally on the fast path — value guards cannot establish
  the branch CONDITION; live repro was functools/argparse corpus flipping
  results on call 2 before this rule). Other regions demoted to the twin,
  reported in `boxed_blocks`.
- **clif.rs** — `compile_tier1` (cold twin = full `compile_baseline` of the
  SAME cfg + typed fast path over plan regions). Typed emitter: native
  iadd/isub (cannot wrap: |ops| ≤ 2^59), mul via exact `h_mul_smi`
  checked helper (imul can wrap INTO the smi range = silent wrong answer),
  div/mod zero-guard before sdiv (twin owns the raise path; sdiv would
  trap), per-result smi-range check (exactness), Python floor-div/mod sign
  fixup inline via block-param SSA merge. Cold-twin seam: baseline block 0
  opens with a `ctx.twin_start` skip chain — region exits park the twin's
  continuation block and hand off (status 2); the twin CONTINUES from the
  exit target (not a restart). The fast path never raises.
- **runtime.rs** — guard/typed helpers (`h_slot_smi_v` sentinel form with
  `GUARD_FAIL_SENTINEL = i64::MIN`, outside the smi domain by
  construction; `h_mul_smi`; `h_store_smi` write-through; `h_return_smi`;
  `h_twin_start`/`h_set_twin_start`; `h_deopt_note`); TIER1 counters +
  `[TIER1-STATS]` probe line; `contains` host seam (`a in b` — same
  inverted-seam pattern as subscript_get).
- **jit_hook.rs** — synchronous tier-up probe at the call boundary:
  snapshot on the calling thread (pon discipline) → hotness gate →
  compile-once → **failed-tier-up-disables** (never retried) → interior-
  mutable `tier1_cell` swap under the shared Rc. Twin-start reset per
  call. `warm_compile_only` (see gate fixes — the warm pass no longer
  executes bodies).

## Gate evidence

- **Typed-region probes measurably faster, byte-identical**: hot typed
  loop (200k iterations, `acc + i*2` shape): tier-0 0.800s vs adaptive
  **0.145s = 5.5x**; stdout identical to each other AND to CPython 3.14
  (39999800000, three-way checked). Debug-build timing; release will be
  measured at 02.05h.
- **Transparency (C-31)**: all 16 tier1-transparency probes + 4 bench
  probes byte-identical tier-0 vs adaptive (rc + stdout + stderr).
  t1_002 mixed-type deopt probe: exact across all three phases (int → str
  flip → int re-specialization).
- **Full corpus mode-pair differential** (286 files): 280 byte-identical;
  the 6 "diffs" are address-print nondeterminism (`<object at 0x...>`)
  that differs run-to-run under the SAME tier — address-masked, all 6
  MATCH. Zero real divergences.
- **Guard side-exits**: deliberate type-flip probe (300 int calls → str
  call in try/except → 300 int calls) → tier-0-identical output; div-by-
  zero deopts to the twin which raises ZeroDivisionError properly
  (guarded BEFORE sdiv — the twin owns the exception path).
- **ap-codegen-native**: 38 tests green (12 new tier1_tests: infer
  lattice, hotness gate, region finder determinism + call-exclusion,
  typed_binop table, compile gates, Risk-5 lockstep).
- **ap-run**: 194/0/1; **ap-gc + ap-obj**: 101/0; **tier_mode_differential**:
  2/2 with the JIT-exercise table now showing **8 JIT-ROUTED probes**
  (compiles=8 fallbacks=2 execs=150959) vs main's all-fallback.
- **Clippy -D clean** on both touched crates (0 warnings, --all-targets).

## Gate fixes (pre-existing bugs this leaf landed en route)

1. **h_call helper ABI arity (02.05b, load-bearing)**: `extra_params`
   over-declared `apjit_h_call` by one (6 vs 5) → EVERY function body
   containing a call failed baseline compile and silently tree-walked
   (fallback-by-table kept bytes identical, hiding it since 02.05a).
   This is why main's [TIER-STATS] showed compiles=0. Fixed + verified.
2. **Warm pass executed bodies (02.05a)**: `warm_module_defs` called
   `jit_call_body`, which RUNS the compiled body — its "NO execution
   happens here" comment was only true while every call-containing body
   failed compile. A module whose top-level short-circuits a call
   (`0 and side()`) printed side effects during warm. Warm now goes
   through `warm_compile_only` (cache-filling, execute-free).
3. **a_ptr_eq missing the Obj arm (02.05a)**: `cls is A` was
   unconditionally false in JIT'd bodies → every `is` over objects
   diverged (c004 class-corpus probe). Added `Value::Obj` ptr_eq arm
   (same rule interp.rs uses).
4. **Membership (`in`) was a fail-closed stub**: CMP_IN/CMP_NOTIN now
   route through the host `contains` seam (interp::contains, iteration
   protocol + `__contains__`), errors reconstructed byte-identically.
5. **Body-local slot bridge**: `slot_names_for` carried param names only;
   every non-param binding the JIT'd body made was dropped at sync-back
   (`global X; X = v` silently lost). cfg::Func now carries
   `local_names` (authoritative name-per-slot record) and the bridge
   syncs body locals.

## Deliberate divergences from pon (recorded)

- **Feedback = hotness gate, not type source** (value guards make the
  typing sound without versions — see Repr decision).
- **Region entry discipline**: pon guards region entry on parameter
  types; we additionally require region.entry == block 0 (fast-entry
  soundness — see optimizing.rs).
- **Cold-twin continuation**: pon side-exits re-run from function entry;
  we CONTINUE the twin at the exit block (`twin_start` seam) so a
  mid-function exit doesn't re-execute already-observed side effects.
  Guard-failure hand-offs still restart (they are detected at loads
  before observable progress).

## Open items (for 02.05d/e)

- `h_deopt_note` counts side-exits but has no epoch/thrash backoff yet —
  02.05d's exact scope (threshold 64, backoff `<<` epoch, pin DISABLED
  after 8).
- Tier-up is SYNCHRONOUS at the call boundary (compile blocks the
  calling thread once per function). 02.05e's background driver +
  pin-until-retire + install-by-CAS replaces this site; the fast path's
  JITModule is currently `mem::forget`-ed (bounded per function) until
  the driver owns retained modules.
- Typed-region OSR remains out of scope (study problem #2, per the
  split); boxed OSR is 02.05f.
- The 02.05g open items resolve as: `gc_value_declarations` stays EMPTY
  **by design** (the i64 repr means no pointer-typed CLIF values exist
  even in tier-1 — the conservative GC floor remains the sole root
  story; documented, not a pending flip). PENDING_MAP_BASE registration
  stays dead code for the same reason. FP retention in the JIT flag
  builder: NOT force-enabled (no pointer-typed values to report; the
  02.05g measurement stands).
