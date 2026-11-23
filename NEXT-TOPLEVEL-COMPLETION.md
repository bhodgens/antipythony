# TOPLEVEL-COMPLETION — campaign tracker (2026-11-22)

Goal: close every remaining named item across Plans 00-05 + 10 so the
top-level plan can be declared COMPLETE. Byte-exact vs CPython 3.14.7
remains the bar everywhere that lowers. Orchestrator: this session.

## In flight (wave 1 + wave 2, all isolated worktrees)

| leaf | delegation | scope | owned files |
|---|---|---|---|
| leaf-1005a | deleg_adfef016 | PyDict/PyList/PyTuple residual (~80-100 members) | ap-ce/src/exports_containers.rs (NEW) | MERGED d8938e5 (ap-ce 19/0, stdlib 48/48 on trunk; 30/30 driver runs byte-exact; framework-linkage SEGV gotcha → wave-2 build-ext lint row) |
| leaf-1005b | deleg_adfef016 | PyObject attr/compare/iter protocol (~60) | ap-ce/src/exports_objproto.rs (NEW) | MERGED afc5abd+cb9506c (ap-ce 27/0, stdlib 48/48, build_ext 4/4 on trunk; 73-pin driver diff empty; PyLong_FromSsize_t duplicate deduped at merge) |
| leaf-1005c | deleg_adfef016 | PyErr plumbing + PyFrame/PyCode projection (~30) | ap-ce/src/exports_errtb.rs (NEW) | MERGED de51151+f249e03 (ap-ce 27/0, stdlib 48/48 on trunk; intermittent abort ROOT-CAUSED = PyErr_Fetch per-call tb value racing the GC, tb slot now None, 20/20 deterministic; both drivers byte-exact vs CPython modulo the 4 recorded divergences; PyObject_GetAttrString + PyLong_FromSsize_t deduped per hazard map; compat-header PyExc macro leak fixed) |
| leaf-harden2 | deleg_adfef016 | HARDEN gap 3: scan-gate gen-condition yield + handler bind-yield | ap-codegen-wasm |
| leaf-02resid-a | deleg_ebe43d03 | V016 parser + t1_007 round + c018 vars() + c032 metaclass | ap-run/{interp,builtins,classes,methods}.rs, ap-parse, corpus pins | MERGED 2db9410 (gates verified first-hand; V016 was already closed on trunk — verification-only; sibling mbc WIP parked at /tmp/sibling-mbc-wip-20261122.patch) |
| leaf-02resid-b | deleg_ebe43d03 | s023 ast.unparse (gate) + Counter repr + CacheInfo + enum mangle + warnings | ap-run/builtins_modules.rs, vendored_stdlib | MERGED f6fb797 (s023 premise stale — already 48/48; leaf landed 5 real fixes: sorted-reverse ties, truthiness, enum marker binding, lru_cache type, warnings exec attribution; gates verified first-hand) |
| leaf-04gaps | deleg_d30ff2d7 | Plan 04 honest gaps: emit-wasm diagnostic, `--` seam, --impl-wasm, hybrid CRT packaging | ap-aot-crt*, ap-conform, ap-cli flag arm, docs | MERGED ced547a (gates verified first-hand) |
| leaf-harden2 | deleg_adfef016 | HARDEN gap 3: scan-gate gen-condition yield + handler bind-yield | ap-codegen-wasm | MERGED 1190808 (gates verified first-hand; GE006 wasm-leg laziness gap filed → 03.04d/e genexp emitter slice) |

Merge order when they land: harden2 first, then 02resid-a/b (ap-run),
then the three ce leaves. Verify-before-merge: re-run each leaf's
headline gates FIRST-HAND in its worktree, then merge, then re-run on
trunk. Gate counts cited by leaves are claims; re-runs are evidence.

## Queued after wave 1+2 merge

0. **build-ext linkage lint row** (small, gates the SEGV gotcha): the
   antipythony-built .so must have ZERO Python.framework load commands
   (otool -L check). The 10.04-era callback.so/payload.so in
   leaf-1004's worktree still carry framework linkage (regenerate via
   build-ext when that lane next runs). Fold this check into
   build_ext_gate.rs as a row, or a scripts/ lint.

1. **Plan 10 wave 2** — DISPATCHED 2026-11-22 late (deleg_6f619bb0):
   (a) leaf-1006 hard members — MERGED 9e36c70 (ap-ce 36/0, stdlib
   48/48 on trunk; longgc+methodmem byte-exact vs committed CPython
   pins; GC conservation 10k/10k; honest gaps: traverse registry needs
   gcmod drain wiring = S2 follow-up; PyMutex global-lock scalability
   note; ~55 members);
   (b) leaf-ge006 genexp-laziness wasm fix — MERGED cf74464 (the
   "eager genexp" framing was a MISDIAGNOSIS; real bugs: plain-fn frames
   never inited the print-buffer local (first print of every user fn
   dropped) + throw-escape left gen state live; 53/53 debug+release,
   42 suites 46/0 on trunk, ap-codegen-wasm-only diff; NEXT-GE006.md
   corrects the 03.04d/e records);
   (c) leaf-linecache AOT source embedding — MERGED 4d8a1d3 (465-file
   include_bytes store, dead-strips when unreachable [hello 0-byte
   delta], disk wins when present; binary-mode slice 46/48 with the 2
   fails = documented interpreter-mode gaps; standalone enum+warnings
   import face verified first-hand via ANTIPTYHONY_STDLIB redirect;
   T-09 30/30, ws 663/0; rebased onto 971399b, additive diff).
   WAVE 2 COMPLETE — all 3 merged.
   After wave 2: pydantic-core probe -> Plan 05 T3-T5 verdicts.
2. **pydantic-core probe** — MERGED e6d249a (2026-11-23). VERDICT:
   **GREEN — kill-criterion #3 does NOT fire.** 110-symbol residual
   demand landed as wrapper members (exports_pydantic.rs, ~1400 lines);
   import pydantic_core + FromSpec Model + one-field validation through
   the built binary RC=0 deterministic (orchestrator re-ran 3/3).
   Honest scope: rebuild-SIMULATED pydantic_core (cp314 binary wheel
   dlopen stays locked-out policy); CPython-arm byte diff of the new
   members = named follow-up. Gate-found fixes: Bytes/Set/Frozen
   from_value arms, module-fn self contract, FromSpec tp_new args
   position. T3/T4 pydantic reachability on S1 STANDS.
   Plan 05 T3-T5 unblock ONLY on a green probe.
3. **Plan 04 honest-gap leaf** — MERGED ced547a (2026-11-22).
4. **Plan 05 T3-T5** (gated on pydantic probe): hermes-agent T3 chat
   turn, fastapi T4 hello-world + TestClient, comfyui T5 load+run.
   T5 additionally needs Plan 10 depth beyond S1 (torch stack) — expect
   a documented reach verdict, not a green.

## Explicitly NOT in scope (locked)

- 02.10 stage 2d — explicitly optional, skipped.
- s016 warning frame LINE (parser-lineno precision family) — documented
  residual, staged-source degradation envelope.
- qualname tb-frame shape — 02.03 envelope, documented residual.
- T121 — CLOSED (T121-fix lock-free bookkeeping + 02E scheduler 2b/2c
  landed; RELEASE-pinned gate >=1.0x green).

## Completion definition for the top-level plan

- Plans 00, 01: COMPLETE (existing).
- Plan 02: 02.08 stdlib slice 48/48 + audit gap list dispositioned
  (leaf-02resid-b) + post-gate native divergences closed (leaf-02resid-a).
- Plan 03: exit gate PASSED (03.07) + HARDEN gap 3 closed (leaf-harden2)
  + extended-corpus trio closed (leaf-02resid-a).
- Plan 04: leaves landed + honest-gap leaf merged.
- Plan 05: T1/T2 complete; T3-T5 verdicts recorded (green or documented
  reach-blocker per tier).
- Plan 10: S1 wrapper band landed + pydantic-core probe verdict recorded.
- Final: workspace gate re-run on trunk (ONE heavy lane, -j4), pytest
  suite green, notices --check, AGENTS.md status block updated, push.
