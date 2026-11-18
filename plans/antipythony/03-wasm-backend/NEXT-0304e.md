# Leaf 03.04e session record (2026-11-17/18, tier1-0304e off main@51a93f62)

## What landed (3 commits: 3e9ce297, f340f214, 1482caa4)

**The SEGMENT MACHINE flattener (NEXT-0304d design) is IN and working.**
GE001 + GE002 run byte-exact through ap-host. The emitter shape (one
DELIBERATE divergence from the manual's per-loop Loop wrap — soundness):

- Nested open Blocks B(0)..B(n-1) + br_table dispatch, exactly the manual.
- ONE dispatch `Loop` around the whole machine (a TRAMPOLINE) instead of a
  Loop per loop-range: a `br_table` resume cannot reach segment bodies
  sitting inside a per-loop Loop label (dispatch runs before the Loop
  opens). Back-edges set the state register and br to the Loop label; the
  machine re-enters dispatch. Every resume point stays reachable; no
  static/runtime label-stack divergence.
- A `$sink` Block between `$done` and the segment blocks: the br_table
  default (state = n) must land ON the done region, not past it. The done
  region FALLS THROUGH `$done`'s End (no Return): dispatch arrives with
  the br_table i32 auto-popped, a resume arrives with an oref payload —
  Return would type-require a value. The FT filler after Loop's End meets
  the (result oref) contract.

Segment kinds implemented: Seq / Test / Park{Bare|Bind|Split} / Rebind /
SplitResume / ForInit / ForNext / YfNext (park-to-self) / YfDone /
Goto (trampoline) / HandlerPrefix (GE003 wrapper) / Return. GE002's
`total + (yield total)` = Park kind=Split(Add, lhs, slot) + SplitResume —
NO expression-level yield evaluation anywhere. Hidden slots
(`__ap_for_it<N>` per For, `__ap_yf_it` shared) ride $gen.$locals via
gen_hidden_slots — gen_slot_count AND the step-fn slot_table append the
same list in the same order.

**Label math (the single biggest time sink — verified formulas):**
at segment k's BODY level (after End(Bk)), open labels innermost-first:
B(k+1)=0 ... B(n-1)=n-k-1, $sink=n-k, $done=n-k+1, Loop=n-k+2.
- forward goto t (k<t<n): br (t-k-1)  → End(B(t-1)) → seg t body
- done escape (t>=n): br (n-k)
- trampoline (t<=k): state reg + br (n-k+2)
- +1 per wrapper label entered at the branch site (If=+1, $cx+try_table=+2
  inside a GE003 wrapper, $chk-Block+If=+2 in ForNext/YfNext).

**En-route fixes (all real bugs, several pre-03.04e):**
1. gen_throw/gen_close call_indirect used the RAW $code.$func_idx as a
   TABLE index — gen_send subtracts FN_FIRST_USER, they didn't. The
   element segment stores absolute fn indices. (GE001's e2.close() trap.)
2. Table minimum n_user+1 (spare entry for the drivers' indirect calls).
3. c.send/close/throw method arms in emit_call's Attribute path — the
   generator object has NO attrs dict, attr_get answered null and the
   $function cast trapped. (Drivers FT_GEN_SEND/THROW/CLOSE now reached.)
4. Step-fn locals pad: `used = 12 + n_slots` (params + state + payload +
   slots + 8-scratch). WAS `2 + n_slots` — the entire shared temp window
   was shifted +10 in every step fn (local 40 was an oref, not print's
   i32). Latent since 03.04d; GE004's delegate print was the first print
   inside a step fn and caught it.
5. gen_park_yield clears $gen.$exn (a staged GeneratorExit must not
   outlive the resume that consumed it).
6. ForInit wraps the delegate expr in FN_CALL_OF: a generator delegate
   lowers to the parked $function object; call_of materializes the
   $generator (plain values pass through).
7. ForNext/YfNext: iter_next's result consumed INSIDE the try_table
   (BlockType::Empty must end stack-empty — the old shape left the result
   on the stack = "values remaining" validate error).
8. gen_emit_call: exception/user-class arm (raise ValueError('mid')
   inside a gen body previously fell to the generic dispatcher →
   emit_function_ref → $function cast on a $type = trap).
9. Test cond evaluated BEFORE the If (wasmtime alloc-in-If wedge, 03.02c
   lesson), spilled to scratch.cmp; Test fall-through tail = unreachable.
10. Loop tails ALWAYS emit the back-chain goto (an early dead-tail
    suppression dropped the br entirely = fall into the done region).

## Gate status vs the leaf exit contract: GE001/GE002 MET; GE003/GE004/GE005/MA001 NOT MET

Honest state at 1482caa4 (verified verdicts):
- GE001-yield-next-exhaust: PASS (byte-exact incl. close/throw surfaces)
- GE002-send-values: PASS (send plumbing + Split/SplitResume + SI value)
- GE003-throw-close: MISMATCH — "1/caught mid/1/closed ok/B 0" then
  UNCAUGHT KeyError('boom') from the module. The handler-yield flow:
  the `inbody()` try-body park (yield 0) + handler yield ('recovered')
  path. Diagnosis pointer: the wrapper's per-segment try_table re-open
  wraps the try-BODY segments; the handler chain runs after $cx End at
  chain level. The KeyError ESCAPES the wrapper — the handler chain's
  EXC_MATCH against KeyError fired? Trace: B 0 printed (first body park
  worked, resume worked), then the raise KeyError('boom') on the SECOND
  body entry (after resume at post-park segment)... the post-park segment
  rides INSIDE the wrapper range (its segment id < after) so its
  re-opened try_table should catch — verify the resume dispatch actually
  re-enters the wrapped region with the handler chain ARMED (the resume
  lands at the segment's $cx open — it does by construction) — most
  likely the handler PREFIX yield parks at the HandlerPrefix segment but
  the resume of THAT park must route back INTO the wrapper's post region
  (resume.next = E_PEND → loop-top/plan-end, NOT back into the wrapper's
  after-region continuation). `B recovered`/`B end` missing = the
  handler-yield value never reached the for-loop: check YfNext-free path:
  HandlerPrefix parks with park value 'recovered', resume = Seq(post)
  with E_PEND — for the for-loop driver the RESUME should continue to
  the next statement INSIDE the handler (post stmts), which it does
  (Seq(post) is the resume segment)... the KeyError escape suggests the
  FIRST body park's resume went to the WRONG segment (skipped the
  `raise KeyError` arm or lost the wrapper). NEXT SESSION: bisect with
  a minimal probe `def h(): try: yield 0; raise KeyError('b') except
  KeyError: yield 'r'` + for-loop driver.
- GE004-yield-from: HOST-ERR — trap chain fn 37 (binop?) ← step ← gen_send
  ← _start. The YfNext SI arm extracts args[0] into scratch.b then
  YfDone binds; the trap is EARLIER (first resume of `delegate()`).
  Suspect: the YfNext park-to-self resumes at the SAME segment, whose
  re-entry re-runs the $cx region (fine) — but scratch.a/x spill: on
  resume the segment re-dispatches and iter_next continues (it-slot
  carries the cursor) ✓... trap in BINOP suggests the `r` bind or the
  print('sub returned', r) arg eval — r rode scratch.b from the SI arm;
  if the delegate's SI fired through iter_next's gen_send path, the
  payload's args[0] extraction (gen_extract_si_payload) reads
  $excinst.args tuple — verify the SI raised by gen_send carries
  args=[$value] with $value = sub's return (42).
- GE005-comprehensions-genexp: LOWER panics — `Named(y, x*10)` (walrus
  in comp) reaches gen_emit_expr's unreachable arm. The comp hidden-fn
  flip (yield-elt, is_generator=true, call sites through
  emit_generator_call + FN_LIST_DRAIN / FN_SUM_DRAIN / DictComp
  dict-build + walrus binding the ENCLOSING scope) is NOT STARTED.
  Also `<listcomp>` must be registered as a module name binding at scan
  (module_name registration for the hidden fn is missing — see the
  ge_run_probe comp-min panic "missing '<listcomp>'").
- MA001-patterns-slice: match EMIT arm not started (scan gates from
  03.04c are in). Design in NEXT-0304d.md: FN_EQ literal chain +
  ArrayLen + FN_DGET str-key + null check + Starred rest via list_drain
  + Or sequential + Guard truthy-after-captures, one Test-chain per case.

## Baselines held at 1482caa4 (re-verified this session)

controlflow_wasm_gate 18/18, classes_exceptions_wasm_gate 17/17,
for_probe 5/5, ap-host 3/3.

## Debug tooling that worked (continue using)

- ge001_bisect.rs (PROBE_CASE=b1..b6) — statement-slice bisect; PROBE_DUMP
  writes /tmp/<case>.wasm.
- dump_wat_resilient (DIAG_SRC/WAT_OUT) + grep @offset — offset-anchored
  wat reading found every bug above.
- The br depth = static count of open labels at the branch site; when a
  "branch depth too large" appears, recount the ACTUAL wat labels
  (wasmprinter annotates `;; label = @N`) rather than re-deriving.

## Machine policy

One cargo lane; CARGO_BUILD_JOBS=4; worktree ~/.git/worktrees/leaf-0304e
(branch tier1-0304e). Probe crates under crates/ap-codegen-wasm/tests/
(minwh.rs, ge001_bisect.rs, val_diag.rs) are throwaway bisect harnesses —
delete or keep as diagnostics, never gates.
