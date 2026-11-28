# fp-rollout 03 — per-function tier dispatch seam (NEXT)

Leaf `fp-tier-dispatch-seam`, branch off main `468e24d`. Prerequisite for the
USER-GATE default flip (`01-adaptive-toggle.md`); **no default was flipped**.
This leaf owns only `ap-run/{jit_hook,interp,lib}.rs` (dispatch/tier plumbing),
`ap-cli/{src/main.rs,src/bin/ap_run_eval.rs}` (flag plumbing), one new test file
`crates/ap-run/tests/tier_seam_differential.rs`, and this doc.

## What landed

`TIER_MODE` is still one thread-local (`jit_hook.rs`), and the bytecode arm
still precedes the JIT arm in `call_function`. The blocker the design brief
pinned (`docs/fast-path-rollout-matrix-design.md §2/§5`) is removed by an
**explicit per-function override** resolved by *function identity*, not source
markers:

- New public `jit_hook::SeamTier { Bytecode, Adaptive, Tree }` and
  `EffectiveTier { Tree, Bytecode, Adaptive }`.
- `jit_hook::tier_override(&FnEntry)` looks up `Arc::as_ptr(&entry.def)` —
  the same stable identity both the bytecode and JIT caches already use — in
  a process-global `OnceLock<HashMap<usize, SeamTier>>`.
- `call_function` (`interp.rs`, the `ObjKind::Function` route) resolves ONE
  effective tier per call: explicit override if present, else the run-wide
  `TIER_MODE`. The bytecode/JIT arms are then selected by that effective
  tier, so a bytecode-dispatched `outer` can call an adaptive `mid`, which
  can call a bytecode `inner`, in ONE process. Tree0 control unchanged.
- `install_tier_seam_for_registry` resolves the spec against the freshly
  built module registry (`FnRegistry::entry_ids` / `entry`, made public for
  this seam). Ambiguous or unknown names are refused; a bad spec produces
  `ValueError` + rc=1, never a silent fallback.
- No override spec ⇒ zero table, zero atomics on the dispatch hot path,
  byte-for-byte unchanged one-mode-per-run behavior (every existing gate).

### Spec syntax (explicit; no heuristic)

```
APX_TIER_SEAM_OVERRIDE="bytecode:outer,adaptive:mid,bytecode:inner"
```

Comma-separated `tier:name` entries; `tier ∈ {bytecode, adaptive|jit,
tree|0}`; `name` = a plain module-level function `__name__` resolving to
exactly ONE registered def (nested/same-name defs are rejected as ambiguous).
The table is process-global and immutable after install (all worker threads
read it). `--tier-seam "<spec>"` on `ap-run-eval` and `antipythony run` simply
sets this env on the interpreter thread before `run_module`. `--tier` parsing
was extended in `ap-run-eval` (adaptive/jit alias) and in `antipythony` to
accept `adaptive`/`jit`; `--tier tree0` remains the control, and the default
is still Tree (`jit_hook.rs:83-84`, `:143`). No source-marker parsing.

## How to run the mixed probe

Fresh release binaries (verified mtimes, one target dir):

```
cd /Users/caimlas/.git/worktrees/leaf-fp-tier-seam
env -u APX_TIER_STATS -u APX_TIER CARGO_BUILD_JOBS=4 \
  CARGO_TARGET_DIR=$PWD/target-fp-seam cargo build --release -p ap-cli \
  --bin antipythony --bin ap-run-eval

BIN=$PWD/target-fp-seam/release/ap-run-eval
# probe: outer(bytecode) -> mid(adaptive, bench_a1 int-while) -> inner(bytecode)
env -u APX_TIER_STATS -u APX_TIER APX_TIER_STATS=1 APX_TIER_SEAM_STATS=1 \
  "$BIN" --tier bytecode \
  --tier-seam 'bytecode:outer,adaptive:mid,bytecode:inner' PROBE.py
```

Inverse: `--tier-seam 'adaptive:outer,bytecode:mid,adaptive:inner'`.
Control: `--tier tree0` (or `--tier 0`) with no seam spec.
Gate: `cargo test -p ap-run --release --test tier_seam_differential`.

## Observed stats (measured, not asserted)

Forward (`bytecode -> adaptive(tier1) -> bytecode`, 30×2000 warmup):

```
[TIER-SEAM] installed 3 per-function overrides: inner=bytecode,mid=adaptive,outer=bytecode
1999003
[TIER-SEAM-STATS] bytecode_execs=62 adaptive_execs=31 tree_execs=0 | inner=bytecode@... | mid=adaptive@... | outer=bytecode@...
[TIER-STATS] compiles=1 fallbacks=0 execs=31
[TIERUP-STATS] tier0=0 queued=0 tier1=1 deferred=0 disabled=0 retained=1
[BC-STATS] compiles=2 fallbacks=0 execs=62 ic: ghit=33 gfill=60 ahit=0 afill=0 chit=0
```

Inverse (`adaptive -> bytecode -> adaptive`):
`bytecode_execs=31 adaptive_execs=62 tree_execs=0`, `tier1=1`,
`BC-STATS execs=31`, `TIER-STATS execs=62`.
4-thread Barrier variant (30×2000 warm-up + 4×3×200 threaded):
`bytecode_execs=84 adaptive_execs=42 tree_execs=0`, `tier1=1`,
`BC-STATS execs=84`, `TIER-STATS execs=42`.
All three stdout/stderr/rc are byte-identical to CPython 3.14 and to tree0;
`tier1>0` proves the adaptive tier installed a genuine tier-1 fast path (not
just a baseline JIT twin), so the byte compare is not vacuous.

`APX_TIER_SEAM_STATS=1` (or `APX_TIER_STATS=1`) emits `[TIER-SEAM-STATS]` with
per-tier exec counts and per-function assigned-tier rows; `[TIER-SEAM]` is an
install-time line. Both are opt-in and absent on default runs.

## Composed matrix cells now reachable

The design brief listed `bytecode × tierup ON` as empty/blocked. With the
explicit override it becomes reachable for any function-shaped chain:

| run-wide mode | per-function overrides | reachable composition | threads |
|---|---|---|---|
| `tree0` (control) | none | tree only | 1 / 4 |
| `bytecode` | none | bytecode only (legacy) | 1 / 4 |
| `adaptive` | none | adaptive/tier-1 only (legacy) | 1 / 4 |
| `bytecode` | `bytecode:outer,adaptive:mid,bytecode:inner` | **bytecode → tier1 → bytecode** | 1 / 4 |
| `bytecode` | `adaptive:outer,bytecode:mid,adaptive:inner` | **tier1 → bytecode → tier1** (inverse) | 1 / 4 |
| `adaptive` | mixed `bytecode:`/`adaptive:`/`tree:` rows | any per-function mixture, including a deliberate `tree:` island | 1 / 4 |

`APX_TIER=bytecode` + `APX_TIER_SEAM_OVERRIDE` works identically via env
(the differential test uses this path through `ap-run-probe`).

## Unsupported shapes / honest gaps

- **Nested / lambda / generator functions**: the override spec resolves only
  MODULE-LEVEL defs by exact `__name__`. Nested defs, lambdas, and generator
  bodies (which the bytecode arm declines by design) are not addressable;
  an ambiguous/unknown name fails loudly, never silently misroutes.
- **Tier-1 fast path requires a tier1-able body**: an adaptive body with no
  typed region (e.g. a pure `return inner(n)` forwarder) runs the JIT baseline
  (`[TIER-STATS] execs>0`) but `[TIERUP-STATS] tier1=0`. The mixed-chain gate
  deliberately uses the `bench_a1` int-while shape so `tier1>0` is asserted.
- **AOT bodies always win**: `aot_runtime::entry_for` is still the first
  dispatch arm, so an AOT-registered function is never tier-overridden.
- **Module-level code** never routes through `call_function` (same as all
  prior tier gates). The chain must be entered via a function call.
- **`--tier tree0`** is accepted as a spelling of `--tier 0` only by the
  existing gate conventions; the control in the new test uses `--tier 0`.
- **One mode by default**: with no spec, behavior is unchanged; the default
  flip remains the USER GATE and is deliberately NOT in this commit.

## Verification (all fresh, env -u APX_TIER_STATS -u APX_TIER,
## CARGO_BUILD_JOBS=4, CARGO_TARGET_DIR=$PWD/target-fp-seam)

- `cargo test -p ap-run --release --test tier_seam_differential`: **3/3**
  (forward, inverse, 4-thread; plus bad-spec loud-failure check in the same
  file — 3 tests total, all green).
- `cargo test -p ap-run --release` (all 20 suites): **225 passed / 0 failed /
  1 ignored**.
- `fix_bytecode.py --other <fresh release antipythony>`: **13 cases / 0
  mismatches**.
- `ap-conform run --corpus 'tests/corpus/values/*.py' --impl '<bin> run'
  --tier 0`: **50/50**; `--tier bytecode`: **50/50**.
- `ap-conform run --corpus 'tests/corpus/stdlib/*.py' ... --tier 0`: **48/48**;
  `--tier bytecode`: **48/48**.
- `cargo test --release -p ap-cli --test tier_mode_differential`: **3/3**.

Debug helper bins (`ap-run-probe`, `ap-run-eval`) were built into the same
target dir before the release suite, because the fenced differential harness
spawns `debug/` helpers (the existing suite convention).
