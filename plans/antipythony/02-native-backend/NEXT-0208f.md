# Leaf 02.08f session record — 2026-11-19 (worktree ~/.git/worktrees/leaf-0208f, branch tier1-0208f off ce01763)

## Status: stdlib slice 47/48 (target met), gates green (1 pre-existing wasm red), T121 re-verify = RED finding (regression, NOT this leaf)

## Commits (branch tier1-0208f)
- 7cff2e2: return-ann attrs self-deadlock fix (s026 typing import hang; same
  MutexGuard-temporary-across-relock class as the iter(genexpr) wedge) +
  __type_params__ + bound __annotate__ marker
- aa6acf0: ap-conform --tier forwarding order (INSERT after program before
  verb; appended form was rejected by the CLI)
- 9711381: typing/protocol surfaces to byte-exact (s026 GREEN)
- 3238696: s020 inspect GREEN + iter(callable, sentinel) + annotationlib
  __globals__ resolution + getmembers dunder-marker skip + FrameInfo
  positions fallback

## Audit gap-list disposition
- iter(<genexpr>) wedge: VERIFIED already fixed on main (cca3eaa + f874aba);
  leaf task A closed verification-only.
- s026 typing hang (NEW deadlock #2, return_ann guard): FIXED -> byte-exact.
- s020 inspect: FIXED -> byte-exact (signature(abs) text-signature path,
  annotation SOURCE recording/resolution, getmembers object-virtual-marker
  skip, getouterframes positions fallback).
- ap-conform --tier forwarding: FIXED (was unusable; now INSERT-after-program).
- T121 quiet re-verify: **RED 0.74x** — see T121-REVERIFY-0208f.md. Serial
  leg got 8.7x faster since 02.09b (0.992s -> 0.114s), parallel only 2x
  (0.300s -> 0.154s); ratio gate regressed. NOT this leaf (no threading/
  scheduler code touched; reproduced on clean main aaafc6d). Needs a bisect
  follow-up leaf.
- s016: ONLY slice red remaining — warning TEXT matches CPython; the frame
  LINE differs (multi-line list display attributes to display start, line 3
  vs CPython line 21). Parser-level lineno precision; same staged-source
  degradation family. Out of leaf scope, documented.

## Gates (all measured this session, this worktree)
- ap-conform stdlib slice --tier 0: **47/48** (was 45/48 at leaf start,
  30/48 at audit start). Target met (48/48 stretch not reached; s016 is
  parser-lineno, not stdlib semantics).
- cargo test -p ap-run --release: 18 suites ok, 0 failed.
- cargo test --workspace --release: 23 suites ok / 0 failed + 1 suite
  (corpus_wasm_sweep) FAILED — VERIFIED PRE-EXISTING on clean main aaafc6d
  (same 5 exceptions/e0xx MISMATCH+HOST-ERR rows; wasm codegen untouched by
  this leaf; also red in sibling worktree leaf-0304g).
- ast_getattr_identity_fence: 2/2 (after copying release probe binaries
  into target/ per the AGENTS.md worktree gotcha).
- pytest suite not re-run in full this session (slice + unit gates cover
  the touched surfaces; corpus runs were the leaf's graded output).

## Files touched
- crates/ap-run/src/{interp.rs, builtins.rs, builtins_modules.rs,
  classes.rs, methods.rs, importer.rs}, crates/ap-obj/src/dict.rs,
  crates/ap-run/src/iter.rs (new CallFn state)
- vendored_stdlib/{typing.py, annotationlib.py, inspect.py}
- crates/ap-conform/src/main.rs (--tier forwarding)
- plans/antipythony/02-native-backend/{NEXT-0208f.md (this file),
  T121-REVERIFY-0208f.md}
