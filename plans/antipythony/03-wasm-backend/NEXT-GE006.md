# Leaf GE006 — genexp wasm-leg laziness (the HARDEN2-disclosed residual)

**Worktree:** `~/.git/worktrees/leaf-ge006` (branch `tier1-ge006` off
main@663c216)
**Date:** 2026-11-22/23
**Goal:** generators_wasm/GE006 passes the CLI corpus gate 53/53; native
leg untouched (it was already byte-exact).

## The disclosure (NEXT-HARDEN2 honest gap #1)

GE006 failed the wasm CLI-corpus leg at BASE. Diagnosis said "the genexp
body evaluates EAGERLY in the wasm emitter's hidden-fn step state — the
`produce N` prints missing". That diagnosis was WRONG about the mechanism.

## Probe-first root cause (two-part — neither part was "eager genexp")

Probe matrix (b/c/d/e/g-series, `ge006_bisect.rs` — throwaway, deleted):

1. **b5** `yield f()` in a plain generator: `f`'s `print` LOST, return
   value CORRECT. **d-series**: ANY first `print` inside ANY plain user
   function frame vanishes; the SECOND print emits (d4: `a`,`b` → `b`
   only; d1/d2/d3/e1/e2/e3/f1/f2 same pattern; module-level prints fine,
   c3 green). NOT generator-specific, NOT genexp-specific.
2. **g2** post-throw `next()` returned the next yield instead of the
   default: `k.throw(...)` caught at the caller, then `next(k,'exh')`
   → `2` (CPython: `exh`). The generator survived a frame-escaping
   exception.

### Root cause 1: plain user-fn frames never init the print-buffer pointer

`emit_user_fn` (lower.rs) never executed
`i32.const PRINT_BUF; local.set 40`. `_start` (lower.rs:1766) and the
generator step fns (lower.rs:3235, the 03.04g fix) both do; plain user
fns never did. Wasm locals zero-start, so the FIRST `print` of every
user-fn frame ran PRINT_ONE(buf=0, s) — bytes written at mem[0] — then
PRINT_FLUSH(1024, 1) computed iov len `1-1024` = huge underflow and
fd_write silently failed. The emitter's post-flush reset
(`local40 = 1024`) made every SECOND print work, hiding the bug behind
"first print only" and costing hours of wrong-cause hunting (string
offsets, EH stash dance, frame_line were all cleared by wat-level
bisects before the local-40 trace closed it).

**Fix:** prologue init in `emit_user_fn` — every frame now addresses the
shared print buffer identically (the documented uniform-convention
contract for locals 40..52).

### Root cause 2: an exception escaping a generator frame left it live

gen_throw/gen_close stage `$exn` and resume the step fn; the machine-wide
try_table catches it; when NO handler matched (including "this gen has no
try at all") the exc was rethrown to the caller — but `$gen.$state` still
held the park's successor. CPython 3.14: an exception propagating out of
a generator frame CLOSES the generator (PEP 342 throw semantics), so a
later `next()` must answer exhausted (the `next(k, default)` default).

**Fix:** in `emit_gen_step_fn`'s close-chain (both rethrow paths share
one site): before rethrowing, if the catch payload is non-null, set
`$gen.$state = GS_DONE`. The handler-MATCHED path (GE003 inbody:
`try: yield` with `except: yield` catching the throw-in and continuing)
is untouched — g3 probe confirms `1 / r / 2` still byte-exact.

Native leg: ZERO native code touched (ap-codegen-wasm only) — byte-exact
by construction; ap-host native suites 8/8 green.

## Gates (all executed this session, this tree)

- CLI corpus gate: **53/53** debug (`target-ge006/debug/ap-host`) AND
  **53/53** release (`target-ge006/release/ap-host`) — GE006 flips, the
  other 52 stay green.
- controlflow_wasm_gate 18/18 (suite: 1 test, 18 cases inside) PASS.
- classes_exceptions_wasm_gate 17/17 PASS. for_probe 5/5 PASS.
- ge_ma_probe PASS (GE001-005 + MA001).
- ap-host 8/8. ap-cli wasm_build_gate 3/3. scan_vet_contract 2/2.
- `cargo test -p ap-codegen-wasm`: 43 suites ok / 0 failed (baseline 42
  + the throwaway ge006_bisect, deleted after).

## Honest notes

1. The HARDEN2/NEXT-HARDEN2 "eager genexp hidden-fn" framing was a
   misdiagnosis; the actual bugs (print-buffer init, throw-escapes-state)
   are recorded here so the 03.04d/e session records stop carrying the
   wrong hypothesis. The genexp hidden-fn WAS correctly lazy — b2's
   `created` before `produce 0` ordering was already right; the prints
   were being dropped by the broken flush, not skipped.
2. Throwaway probes (ge006_bisect.rs, logs-ge006/) deleted before commit;
   the two repro pins (b5-equivalent, g2) are COVERED by GE006 itself in
   the corpus.
3. The d-series "first print lost" bug likely explains scattered older
   "print silently missing" notes in 03.04d/e records (GE003 'closing',
   GE004 prints) — those were attributed to step-fn local-40 init (real,
   fixed 03.04g) but PLAIN fn frames had the same hole.
