# NEXT-1004FIX — Leaf 10.04fix session record (post-merge ce regression)

Date: 2026-11-22. Worktree: `~/.git/worktrees/leaf-1004fix` (branch
`tier1-1004fix`, off main `6a47530`, the tier1-1004 merge). One commit:
`e02fcc8`.

## VERDICT: real bug, NOT stale rlibs — and NOT in 1004's features

Task 1 executed first per the brief: `cargo clean --release -p ap-run
-p ap-ce -p ap-cli` in the MAIN checkout, full release rebuild, re-run of
the repro (`leaf-1002/tests/ce-spike/spike_check2.py`) → still `0`. Build
artifacts ruled out.

Actual root cause: the `6a47530` conflict resolution of
`crates/ap-ce/c/apx_shim.c` `PyArg_ParseTuple` left `case 'i'` consuming
the `va_arg(ap, int *)` but **never writing `*out`** — the entire 10.03
conversion body (PyLong_AsLong + INT_MIN/INT_MAX overflow check +
`(int)v` store) was dropped, replaced by a bare fetch+break. The
"bisect 1004's changes" framing was a red herring in one respect: the
bug isn't in runtime_ctx/payloads/value_conv dispatch at all — it's a
shim text regression introduced AT merge time. Why leaf-1004's own
binary printed 42: its final tree (be27276) had `case 'i': case 'l':`
SHARING one body that wrote a full `long` — correct for spike.c only
because spike.c happens to declare `long` receivers. The merged text
took 10.03's `int *` receiver (so spike.c's longs would be half-written
even if the store had survived) AND lost the store entirely, so
`a`/`b` stayed at their `= 0` initializers → `add_two` returned 0. The
string path (`greet`) was untouched, matching the observed signature.

Secondary find: be27276's merged-shim `'l'` inherited 10.03's
int-narrowing + OverflowError at INT_MAX — a silent CPython divergence
for `'l'` (CPython 'l' stores a full long, overflow only past LONG
range). payload.c's `long` receivers passed gates only because no probe
exceeded INT_MAX. Fixed as part of this leaf: `'l'` now stores the
untouched long.

## The fix (e02fcc8, crates/ap-ce/c/apx_shim.c only)

- `case 'i'`: CPython getargs.c f_i semantics restored — `int*` out,
  PyLong_AsLong, OverflowError `"Python int too large to convert to C
  int"` past INT_MAX, store `(int)v`. (10.03's fix preserved exactly.)
- `case 'l'`: CPython 'l' semantics — `long*` out, PyLong_AsLong (its
  -1+PyErr_Occurred IS the overflow signal), store `v` unmodified.
- 1004 features untouched: `git diff 6a47530..e02fcc8 --stat` = shim.c
  only (+20/−8). runtime_ctx, payloads, value_conv, value_map,
  ce_bridge, exports all byte-identical to main.

## Gates (measured, this worktree, target-1004fix)

- spike_check2.py (42 / hello, antipythony!): byte-exact vs CPython
  3.14.7 (stdout+rc, fenced) under RELEASE **and** DEBUG.
- 1004 proof extensions, fresh `build-ext` + cc dual-compile in a clean
  scratch dir: callback_driver.py **R1–R9 byte-identical** vs CPython
  (call-back bridge intact); payload_driver.py **P1–P7 byte-identical**
  (payloads intact). spike gate probe (add_two(2,40)/greet/fails/Point)
  byte-identical stdout+stderr+rc.
- ap-ce: ce_s1_units **5/0**.
- ap-run RELEASE: **18 suites, 206 passed / 0 failed / 1 ignored**
  (aprun-suite.log in the worktree).
- ap-conform stdlib `s0*` slice tier-0: **48/48**; exceptions `e0*`
  slice: **18/18** (e016/e017/e018 instrument-line cases green = the
  s016/s016b concern).
- ap-cli stdlib_modules_differential (cargo test --release): **48/48**.

## Main-checkout hygiene

- Never touched the main checkout's mbc files; `git status` there showed
  NO mbc WIP at session start/end (sibling's state was already clean).
- The one accidental main-checkout write: `cargo clean -p ap-run -p
  ap-ce -p ap-cli` WITHOUT CARGO_TARGET_DIR cleaned `target/` (default
  dir, 1.3GiB of stale pre-merge artifacts) — harmless (that dir is a
  leftover convention dir; the live dir target-0304c was then cleaned
  properly with --release and rebuilt from scratch, which is what
  disproof of the stale-rlib hypothesis required).
- A `re_fuzz` seed append landed in the worktree's FUZZ_SEEDS.txt from a
  cargo test run; reverted before commit (corpus only grows — these were
  machine-generated during a gate run, not curated).

## Honest gaps

- Pending sibling lanes NOT re-run here (owned by the orchestrator per
  NEXT-1004's PENDING list): ast_getattr_fence, tier_mode_differential,
  lxml_differential + remaining ap-cli suites.
- MAIN checkout binaries: target-0304c release was rebuilt clean but
  still contains the BUGGY shim (the fix lives on tier1-1004fix until
  merge). Anyone re-running the repro against
  `target-0304c/release/antipythony` before merging this branch will see
  `0` again — that is expected, not a recurrence.
- Release per-call pycall latency (report gap #2) still unmeasured.
