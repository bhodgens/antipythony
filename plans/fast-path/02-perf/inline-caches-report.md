# fp-inlinecache2 — inline caches REBASED onto current main

Leaf: plans/fast-path/02-perf/01-inline-caches.md
Branch: tier1-fp-inlinecache2 (worktree ~/.git/worktrees/leaf-fp-inlinecache)
Base: main @ 1897b4a (post fp-exceptions c633a0b + fp-calls + unsafe waves)
Recovered from: the prior fp-inlinecache attempt (62118d8 impl + 83f63bd
tests/probe + bada6b3 report, cut BEFORE fp-exceptions/fp-calls merged).
Supersedes that attempt's inline-caches-report.md (same path — this file).

## What landed (rebase commits)

- 0492106 feat: the prior work's caches.rs / sites.rs (dense by_instr
  pc->site map) / ic.rs / ic_invalidation.rs re-landed; instr.rs
  LoadAttrCall gains its site field; decode.rs prints it; lib.rs wires
  both modules; classes.rs regains the interp-free classifier shims
  (descriptor_kind_pub / is_data_descriptor_scratch).
- 3bb8343 test: ic_invalidation adapted to the current trunk (see gates).
- lower.rs: LoadAttrCall sites allocated in BOTH emission paths (the
  direct obj.name(args) fused call AND comprehension append/add). Note:
  current trunk's `emit_at_line` returns () — the emitted instruction's
  index comes from the new `cur_instr_idx()` helper (last instrs.len()-1).
- bytecode_interp.rs: the three consult/fill arms grafted onto the
  exception-table dispatch loop (fp-exceptions rewrote the loop's
  prologue/unwind; the IC arms are additive per-opcode bodies):

| site | entry stamps | warm hit cost |
|---|---|---|
| LoadGlobal | globals dict epoch + builtins dict epoch + both dict identities, resolved Value | 2 epoch loads + 2 ptr compares + Value clone |
| LoadAttr | class payload ptr + type_version + chain_surgery_version + names-dict epoch, plan + payload | 4 stamp compares + plan answer |
| LoadAttrCall | (receiver tag, name, argc) -> dispatcher decision | 1 argc check (tag re-derived from the live receiver) |

  Rebase-specific adaptation: the LoadAttrCall arm's (tag, name)
  classification now flows through the shared `tag_of()` helper (the old
  inline match duplicated it), and the cached-plan consult replaces the
  per-hit `builtin_value_method_exists_pub` table probe. Fused fast arms
  (list.append/dict.get/set.add from fp-controlflow) still answer BEFORE
  the cache — chit stays 0 on those shapes by design.

## No-GIL shape (unchanged from the prior attempt, re-verified)

Per-thread tables (thread_local keyed by the code-ptr; the compiled-body
Arc pin prevents key aliasing). Warm hits take NO lock and perform ZERO
shared writes. Soundness rides the stamps, not the storage: every hit
revalidates against the LIVE 02.05b counters (Relaxed loads); a stale
entry is a miss and a miss runs the exact slow path. Fills use
try_borrow_mut (a nested frame of the same body mid-dispatch skips the
fill; the site refills later). Cached Values are stored DIRECTLY (never
as raw bits) — the tracing GC keeps their referents reachable through
the conservative floor (the prior attempt's use-after-free lesson).

## Invalidation surface (02.05b bump-site map, re-audited)

- Globals: dict epoch bumps (PyDict::set/pop); pair identity catches
  module swaps. Only provenance-verified answers (present in globals OR
  builtins at fill time) record; NameError negatives stay dynamic.
- LoadAttr: attr_set_protocol UserClass/interned arms + class-attr write
  + delattr all bump type_version/surgery_version (audit rows 10/11/16;
  classes.rs needed NO new bump sites). Subclass chains ride
  chain_surgery_version (MRO max). InstDict re-probes the live attrs
  dict per hit (value rebinds need no counter); BindFunc mints fresh per
  hit with owner re-derived from the LIVE MRO + ptr-checked.
- LoadAttrCall: key is a pure function of the instruction + live stack
  receiver — no mutation surface, no stamp needed.

## Measurement (ic_base.py probe, best-of-5 of 3 runs, M-series host,
## release build target-fp-ic2 @ 3bb8343)

| row | tier-0 (no IC) | bytecode + IC | CPython 3.14 |
|---|---|---|---|
| global_read (C) | 314.3ns | 56.2-58.9ns | 19.4ns |
| attr_inst (c.x) | 571.3ns | 134.7-139.0ns | 21.4ns |
| method_call_dict (d.get) | 3697.8ns* | 136.2-138.8ns | 30.2ns |
| method_bound (c.m mint) | 860.9ns | 850-872ns | 44.0ns |
| method_call_inst (c.m()) | 1457.9ns | 1217-1245ns | 32.1ns |
| mint_dict (d.get bare) | 3319.3ns* | 3314-3848ns | 50.3ns |

(*tier-0 rows ride fast_bound_mint differences — mint floor dominates.)

HONEST, same conclusion as the prior attempt: the leaf-spec absolute
targets (global <=30ns, attr <=60ns) are NOT met. The hit path is ~5
compares + a clone; the remaining cost is the tier-0 dispatch loop floor
(line-table load + stack ops + For overhead). The IC removed the
RESOLUTION cost (global 5.5x, attr 4.1x vs tier-0 same binary); rows are
now loop-dominated. Prior attempt measured 60ns/132ns on its base —
rebased numbers are equal-or-better (56-59 / 135-139); parity within
noise. CPython parity needs the tier-1 dispatch work, out of scope.

Hit-rate evidence (APX_TIER_STATS=1): ghit=1399984 gfill=72
ahit=349999 afill=1 chit=0 — ~1 fill per site per thread then pure hits.

## Gates (all re-run on 3bb8343, clean env, release target-fp-ic2)

- fix_bytecode.py: 13/13 probes three-way GREEN (tw == bc == CPython,
  byte-identical) — the fp-exceptions count, both toggles exercised.
- diff_b64 2132/0; diff_hex 1976/0; fix_int / fix_json 127/0 /
  fix_sort 42/0 / fix_random: GREEN.
- fix_sha_zlib: tw == bc byte-identical (0 diff lines); vs CPython 28
  CORRUPT-row zlib.error message divergences — ALL the documented
  zlib-rs message-granularity residual (fix-sha-zlib-report.md),
  zero new.
- ap-conform values 50/50 + stdlib 48/48, BOTH toggles (default tier-0
  and --tier bytecode).
- cargo test -p ap-bytecode -p ap-run --release: 21 suites, 272 passed,
  0 failed (incl. the 7-gate ic_invalidation suite).
- ic_invalidation.rs: 7 gates x BOTH tiers (tier-0 control + bytecode)
  — set/del/re-add instance attr; set/del/re-add class attr (inst views
  checked too); method add + overwrite; base-override-then-subclass
  override (the chain case); global rebind / builtins shadow /
  unshadow / del (NameError stays exact); fused-call receiver mutation.
  Rebase fixes: binary path points at target-fp-ic2; the clean-stderr
  asserts filter the [TIER-STATS]/[BC-STATS] transparency lines that
  fp-exceptions made unconditional on the bytecode route.
- no-GIL probe: no_gil_4thread_contention — 4 threads x shared sites
  while a mutator churns values + class surgery concurrently, stale-
  FREEZE detection — green 3x stable.
- Toggle-off: --tier 0 runs never enter the bytecode path (unchanged).

## Repair pass — instance-dict shadowing of ClassValue/BindFunc hits
(adversarial audit deleg_2f2f5aa5, REJECT verdict, branch ic2-attempt)

Reproduced on the a0fc09d binary with a HOT function (ahit=3000, afill=1):
`class C: x=1; c=C(); def read(): return c.x` warmed 3000x, then `c.x=42`
→ bytecode **1** (stale) vs tree-0 / CPython **42**; the same shape for a
method (`def read(): f = c.m; return f()` warmed, then `c.m=lambda:...`)
→ bytecode **class-method** (stale) vs **instance-lambda**. The audit's
earlier probes hit 1-2 functions with ahit=0, so they could not show the
hit arm; these probes prove the defect on a genuine IC hit.

Fix (bytecode_interp.rs): a ClassValue/BindFunc plan records that the class
supplies the name AND the instance dict does not shadow it. Instance writes
move no class stamp, so (a) `ic_attr_fill` now probes the live instance dict
once and records NOTHING (clearing any stale entry) when the name is there;
(b) BOTH hit arms re-probe `instance_dict_has(&obj, name)` before serving the
cached class answer and fall through to the exact slow walk when shadowed.
Post-fix hot probes: ahit=2999 with correct 42 / instance-lambda.

No-GIL gate repaired: `ic_invalidation.rs:422` asserted `!out.contains("TEAR")`
while the probe emits `FREEZE` — vacuous. Worker counts are now strict
(each == 200000) and the probe sustains mutation until all workers finish.
Because `TIER_MODE` is thread-local (worker fibers run tree-tier), the gate
was reshaped to hot-read the shared instance dict on the MAIN thread while
four workers mutate it: `[BC-STATS] ahit>0` is asserted, so the IC hit arm is
provably exercised. Two new regression tests (`inval_instance_shadow_after_warm`,
`inval_method_shadow_after_warm`) warm a class member/method in a bytecode
function, shadow it via the instance dict, and assert verdict equality with
tree-0 and CPython plus ahit>0. `run_probe` strips every inherited APX_*
var before setting the tier under test (kills the env-leak false RED).

Repair gates (fresh target-fp-ic2b / target-fp-ic2 binaries, mtime verified,
env -u APX_TIER_STATS -u APX_TIER):
- ic_invalidation: 9/9 (7 base + 2 new).
- ap-bytecode + ap-run --release: 21 suites, 274 passed / 0 failed / 1 ignored
  (272 baseline + 2 new tests).
- fix_bytecode.py --other: 13/13.
- ap-conform values 50/50 + stdlib 48/48, both tier-0 and bytecode.

The fresh target-fp-ic2b debug bins (ap-run-eval/ap-run-probe) were required
for the full battery: the shared target-fp-ic2 debug bin predated the fix and
caused 7 false REDs in fix_pass_differential until rebuilt.

## Honest gaps

1. Absolute ns targets (30/60) not met — dispatch-loop floor dominates
   (loop overhead ~40ns/iter at tier 0). Filed as tier-1 dispatch work.
2. Both no-GIL shapes were not measured against each other (per-thread
   won on design: zero shared traffic); only correctness is probed under
   contention, not throughput scaling.
3. mint_dict/method_bound rows are mint-floor-bound (~850ns-3.8us);
   the BoundMethod mint is the primitive-perf campaign's filed lever.
4. LoadGlobal negatives (NameError paths) uncached by design.
5. LoadAttrCall memo only engages on shapes NOT covered by fp-calls'
   fused fast arms — those skip the cache entirely (they're already
   faster than any cache consult); chit=0 is the intended shape.
6. The 8-failed import_guard_differential run seen mid-leaf was THIS
   shell's stray APX_TIER_STATS=1 env (stats line = stderr last line);
   clean env re-run: 9/9 green on trunk too. No code change needed.
