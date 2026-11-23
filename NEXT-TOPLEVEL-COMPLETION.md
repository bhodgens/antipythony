# TOPLEVEL-COMPLETION — campaign tracker (2026-11-22)

Goal: close every remaining named item across Plans 00-05 + 10 so the
top-level plan can be declared COMPLETE. Byte-exact vs CPython 3.14.7
remains the bar everywhere that lowers. Orchestrator: this session.

## In flight (wave 1 + wave 2, all isolated worktrees)

| leaf | delegation | scope | owned files |
|---|---|---|---|
| leaf-1005a | deleg_adfef016 | PyDict/PyList/PyTuple residual (~80-100 members) | ap-ce/src/exports_containers.rs (NEW) | MERGED d8938e5 (ap-ce 19/0, stdlib 48/48 on trunk; 30/30 driver runs byte-exact; framework-linkage SEGV gotcha → wave-2 build-ext lint row) |
| leaf-1005b | deleg_adfef016 | PyObject attr/compare/iter protocol (~60) | ap-ce/src/exports_objproto.rs (NEW) | MERGED afc5abd+cb9506c (ap-ce 27/0, stdlib 48/48, build_ext 4/4 on trunk; 73-pin driver diff empty; PyLong_FromSsize_t duplicate deduped at merge) |
| leaf-1005c | deleg_adfef016 | PyErr plumbing + PyFrame/PyCode projection (~30) | ap-ce/src/exports_errtb.rs (NEW) |
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

1. **Plan 10 wave 2** (after ce leaves merge): PyLong_Export digit
   layout (~6 members, 4-8h), tp_traverse/GC bridging (~10 members,
   6-12h, conservation-critical), method/class ctors + PyMutex/
   PyUnstable/Py_Version (~25), PyMem/PyGILState/module surface (~40).
   Split into two more leaves: (a) hard members (PyLong_Export +
   tp_traverse + GC), (b) the method/mem/GIL band.
2. **pydantic-core probe** (after wave 2 lands): its 11 non-stable
   symbols all fall in landed bands except the Call family (landed in
   10.04) — probe a trivial model per ce-compat-survey §3. Gate =
   kill-criterion #3 check. Plan 05 T3-T5 unblock ONLY on a green probe.
3. **Plan 04 honest-gap leaf** (disjoint, can dispatch any time):
   plain-CRT --emit-wasm diagnostic (ap-aot-crt), -- separator for the
   hybrid flag seam, ap-conform --impl-wasm seam, hybrid CRT packaging
   note for 04.03 release matrix. Files: ap-aot-crt, ap-conform, docs.
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
