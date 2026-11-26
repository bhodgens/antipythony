# Tier-1 Primitive Dispatch — inventory, deltas, bridges, verdict

Leaf pp2-tier1 (research-and-prototype), 2026-11-25, worktree
`~/.git/worktrees/leaf-pp2-tier1` (branch `tier1-pp2-tier1`, merge base
68c2a14). Question: **what does the existing tier-1 JIT buy on the
primitive bench's walk-floor-bound shapes, and what would it take to
route the remaining hot loops there?**

TL;DR verdict: tier-1 as it exists is an **int-arithmetic while-loop JIT
plus a modest baseline-twin win** — worth 1.6–2.1x on the shapes it can
reach. The primitive bench's remaining FAIL rows are dict/sort/str-loop
shapes that tier-1's typed regions *decline by design*; the actual wins
available today came from **emit fixes that let those shapes reach the
baseline twin at all** (they previously fell back to the tree-walker
entirely), not from typed codegen. Two bridges landed; one new JIT emit
bug found + fixed; one pre-existing silent adaptive-mode divergence
(JIT'd ordering compares skipped the rich-compare protocol) found + fixed.

## 1. Inventory — tier-1 capabilities vs primitive bench shapes

Mechanics (all read-verified in source):

- **Dispatch**: the JIT arm lives in `call_function` (interp.rs →
  `jit_hook::jit_call_body`). Only CALLED function bodies ever reach
  compiled code. Module-level loops NEVER do (`execs=0`) — every
  primitive bench probe must wrap its loop in a function to even be a
  candidate.
- **Tier-1 typed fast path** (02.05c): region finder (`region.rs`
  `block_eligible`) admits ONLY — `LoadLocal/StoreLocal` over unboxable
  temps, smi `Const`, `BinOp{+,-,*,/,//,%}` over unboxable operands,
  relational `Compare`, `Truthy`. Everything else (`Call`, `LoadGlobal`,
  `LoadAttr`, `Subscript`, `StoreAttr`, exception machinery, …)
  disqualifies the block → boxed baseline twin.
- **Hotness gate** (`infer::has_int_hotness`): a binop feedback cell must
  reach MONO(IntI64, IntI64). Dict/str traffic never mono-types int-int
  → never enqueued for typed compile (by design).
- **Baseline twin** (02.05a/02.05h): the compiled boxed lowering every
  function body gets when the lowering supports it. Worth 1.24–1.76x on
  non-int shapes (docs/tier1-bench.md b1/b2 rows).
- **OSR** (02.05f): boxed OSR end-to-end for `while` loops only —
  `Stmt::For` is `Unsupported` in `cfg::lower_fn` (cfg.rs:470), so any
  function containing a `for` loop cannot be lowered AT ALL
  (lower-reject → pure tree-walk, execs=0). Also unsupported:
  `Break/Continue`, `while…else`, tuple-unpack stores, AugAssign,
  `BoolOp (and/or)` (short-circuit), multi-target assign, dict/list
  LITERALS with items.

Shape-by-shape (primitive bench FAIL rows that are walk-floor-bound):

| bench shape | in a `def`? | lowering | typed region | twin win | verdict pre-leaf |
|---|---|---|---|---|---|
| dict.get (`c += g(k)`) | yes (probe style) | while: OK; **for: NO** | declined (Call) | **BLOCKED by emit bug** | 0.99x — pure tree-walk |
| dict.getitem (`c += D[k]`) | yes | while: OK | declined (Subscript) | yes | ~1.0x measured — twin ≈ walk here |
| dict.build (`d[i] = i`) | yes | **lower-reject** (`d[i]=i` was Unsupported; `{}` literal Unsupported) | — | **none possible** | 1.0x — pure tree-walk |
| str.find in loop | yes | while: OK | 1 region installs (its int counter) | partial | ~1.1x |
| str.join / split | yes | `for` loops in bench harness → lower-reject | — | none | pure tree-walk |
| list.sort / sorted | n/a (native Timsort) | — | — | — | not interpreter-bound |
| listcomp | compiles to a for-loop shape | lower-reject | — | none | pure tree-walk |

Known-fallback ledger check (02.05h list, re-verified on this tree):

1. **"Stmt::For at module level never reaches jit_call_body"** — still
   true, and worse than documented: `Stmt::For` ANYWHERE in a function
   body makes `lower_fn` reject the whole body (`Unsupported("For")`).
   The 02.05h bench slice used only `while` loops, which is why its
   numbers looked good.
2. **"builtin Call(local-Name arg) feeding BinOp in while loop fails
   Cranelift verify"** — REPRODUCED and ROOT-CAUSED (see §2.1); it was
   not the Call/BinOp shape at all. FIXED.
3. Float loops decline typed regions by design — unchanged (by gate).

## 2. What this leaf found and fixed

### 2.1 The Cranelift "invalid block reference block3" bug — root cause + fix

The 02.05h doc blamed "builtin Call with local-Name arg feeding BinOp".
Black-box bisection (`tests/bench/primitive/probes/_pp2_minimize1..11.py`)
pinned the real trigger:

- `c = c + D.get(ks[i])` (subscript arg + binop) → FALLBACK
- `c = c + D.get(0)` (literal arg + binop) → COMPILED
- `c = D.get(ks[i])` (subscript arg, no binop) → COMPILED
- `c = c + 1; z = D.get(ks[i])` → FALLBACK

Root cause (unit repro: `crates/ap-codegen-native/examples/repro_verify.rs`,
run with `cargo run -p ap-codegen-native --example repro_verify`):
`loop_headers()`/`osr_plan()` count EVERY local slot into the OSR live
set. The subscript-arg bodies have 17 > OSR_MAX_LIVE=16 live at the
header → the header is OSR-INELIGIBLE. The `OsrPoll` emit arm checked
eligibility AFTER emitting its `brif(has_entry, osr_b, …)` block
structure and then bailed with `return Ok(())` mid-inst-loop — leaving
the current block unterminated while its successors were already
referenced → "invalid block reference block3" at verify, and the whole
body silently fell back to the tree-walker (execs=0, no twin, no win).
Literal-arg bodies have 13 live < 16 → eligible → transfer path emitted
fine. That's why the shape matrix looked semantic when the real variable
was the OSR live-set size.

Fix (clif.rs, `Inst::OsrPoll` arm): **eligibility check FIRST** — an
ineligible header degrades the poll to a bare `h_osr_poll` registry
probe (one helper call, no branch structure), before any block creation.
Regression test: `crates/ap-codegen-native/tests/pp2_osr_verify_regression.rs`
(q1 shape compiles baseline-with-osr; q3 control still compiles).

Effect: dict_get / dict_getitem bodies now COMPILE and execute via the
baseline twin (`compiles=1 fallbacks=0 execs=N`) instead of
`compiles=0 fallbacks=1 execs=0`.

### 2.2 Bridge 1 — dict.build reaches the twin (`StoreSubscript` + `{}`)

`d[i] = i` was `Unsupported("Subscript store")` in `assign_target`, and
`{}` was an unsupported expression form — the dict.build shape
lower-rejected, so no compiled path was even reachable. Landed:

- `cfg::Inst::StoreSubscript(obj, idx, val)` + lowering in
  `assign_target` (region-ineligible by design; boxed protocol only).
- `Expr::Dict` (empty `{}`) lowers as `dict()` (LoadGlobal + Call) —
  same name-resolution path the tree-walker uses.
- `runtime::h_set_subscript` helper (fail-closed when the host fn is
  absent) + `SUBSCRIPT_SET_FN` seam + clif emit arm + helper_symbols
  registration.
- ap-run installs `methods::subscript_set` as the host fn at JIT init.

Effect (probes/pp2_tier1_report.json, 7 runs, median/median,
Mac M2 Ultra, RELEASE): **dict_build 1.0x → 1.82–1.84x** (best-of runs
2.31x in the first post-fix sweep; 1.8x stable across 3 subsequent
sweeps — the conservative number is ~1.8x). The dict-build row's cost
was the walk loop around the inserts, and the twin removes exactly that.

### 2.3 Bridge 2 (found by the gates, fixed) — JIT'd ordering compares
bypassed the rich-compare protocol

Rerunning the gates against the twin-enabled tree exposed a **silent
adaptive-mode divergence**: ap-conform stdlib s023
(ast.parse/dump/unparse) passed tier-0 48/48 but FAILED adaptive on
main's pre-leaf code too (verified: reproduces on the main checkout's
trunk binary) — i.e. a pre-existing tier-1 transparency bug the
mode-pair differential never caught (it only runs the 16 transparency
probes, none of which compare user objects).

Root cause: the JIT'd `h_compare` helper routed ALL ordering ops through
`ap_obj::ops::lt` → `values_cmp`, which has no user-Obj richcompare
dispatch — any `a < b` over user-class instances or IntEnum members
inside a JIT'd body raised `TypeError: '<' not supported between
instances of 'object' and 'object'` where tier-0 (interp::
`compare_values_pub` → user_compare → `__lt__`) succeeded. Minimal repro
(/tmp/s023_min38/51 in the session record; class in doc §4): a JIT'd
`return a < b` over two instances of a plain class with `__lt__`.

Fix: `COMPARE_FULL_FN` host seam — ordering ops in `h_compare` route
through ap-run's `compare_values_pub` (Eq/NotEq keep `ops::eq`; unwired
unit context keeps the static values_cmp arms). ap-run installs the seam
at JIT init.

Effect: s023 now passes adaptive (stdlib 48/48 adaptive AND tier-0;
values 50/50 adaptive). The `tier_mode_differential` corpus (3/3 green)
plus the seven diff gates all stay green.

## 3. Measured deltas (the honest table)

Probe: `tests/bench/primitive/probes/pp2_tier1_dispatch.py` (fixc_dict
isolation pattern; all six workloads in one process, function-wrapped
while-loops where legal, N=32768); harness
`pp2_tier1_harness.py` (7 alternating tier-0/adaptive runs, medians,
byte-identity check across modes, APX_TIER1_STATS install proof).
Machine: Mac M2 Ultra, RELEASE, `--tier 0` vs `--tier adaptive`.

| shape | tier-0 ms | adaptive ms | speedup (3 sweeps) | tier-1 state |
|---|---|---|---|---|
| int_acc (`acc+=i` while) | 13.3 | 6.4 | **2.05–2.09x** | typed region installed (02.05h's a1 row, control) |
| dict_get (`c+=g(k)` while) | 14.0 | 14.0 | **0.99–1.01x** | twin runs (was tree-walk pre-fix); dict.get itself dominates |
| dict_getitem (`c+=D[k]` while) | 9.8 | 9.9 | **0.98–1.00x** | twin runs; dict lookup dominates |
| dict_build (`d[i]=i` while) | 13.0 | 7.2 | **1.82–1.84x** (2.31x first sweep) | twin (bridge 1) |
| str_find (`TEXT.find(w)` while) | 0.96 | 0.90 | **1.07–1.14x** | typed region on the int counter + twin |
| str_concat (`s=s+"y"` while) | 1.3 | 0.80 | **1.64–1.76x** | twin |

Readings, stated plainly:

1. **Typed regions only ever help int-arithmetic while-loops** (2.05–2.1x
   here; 2.07x gate geomean on 02.05h's slice). None of the primitive
   bench's remaining FAIL rows can ever enter a typed region: their hot
   ops are `Subscript/Call/LoadAttr`, which `block_eligible` excludes
   by design. Getting dict/sort/str rows to parity-class requires
   making those ops typed (structural work, §5) — not dispatch work.
2. **The twin is worth ~1.6–2.1x where it runs** (str_concat 1.6x,
   dict_build 1.8x) but **~1.0x where the primitive dominates** (dict
   get/getitem: the twin executes the SAME h_subscript/h_call helpers as
   the tree-walker; the win is only the walk overhead around them, which
   the primitive masks).
3. **The biggest wins this leaf landed were NOT tier-1 wins** — they
   were *reachability* fixes: shapes that lower-rejected or
   verifier-failed now compile at all. The remaining primitive FAIL
   rows stay bound by the same per-op costs as before
   (~985ns registry insert, ~1.5µs BoundMethod mint, memchr-class str
   ops) — those are the levers the primitive-perf report already filed.

## 4. New surface landed (files)

- `crates/ap-codegen-native/src/clif.rs` — OsrPoll eligibility-first
  fix; `StoreSubscript` emit arm; `apjit_h_set_subscript` registration.
- `crates/ap-codegen-native/src/cfg.rs` — `Inst::StoreSubscript`;
  subscript-store lowering; empty-`{}` dict lowering.
- `crates/ap-codegen-native/src/runtime.rs` — `h_set_subscript` +
  `SUBSCRIPT_SET_FN` seam; `COMPARE_FULL_FN` seam + `static_compare`
  fallback; h_compare ordering-op rerouting.
- `crates/ap-codegen-native/src/{region,osr,infer,optimizing,aot}.rs` —
  StoreSubscript in the inst walks (region-ineligible; reads/writes).
- `crates/ap-run/src/jit_hook.rs` — installs the subscript-set and
  full-compare host seams at JIT init.
- `crates/ap-codegen-native/tests/pp2_osr_verify_regression.rs` (new) —
  ineligible-header loop compiles baseline-with-osr; eligible control.
- `crates/ap-codegen-native/tests/pp2_store_subscript.rs` (new) —
  dict-build lowers+compiles; set-subscript helper fails closed.
- `crates/ap-codegen-native/tests/cfg_tests.rs` — the
  `Unsupported("Subscript store")` expectation flipped to a positive
  StoreSubscript lowering assertion.
- `crates/ap-codegen-native/examples/repro_verify.rs` (new) — the
  verifier-bug unit repro (real pipeline: lower_source → lower_fn →
  compile_baseline_with_osr), kept for fast iteration.
- `tests/bench/primitive/probes/pp2_tier1_dispatch.py` +
  `pp2_tier1_harness.py` + `pp2_tier1_report.json` (new) — the
  measurement slice + harness + report.
- `tests/bench/primitive/probes/_pp2_minimize*.py`, `_pp2_iso_stats.py`,
  `_pp2_fallback_diag.py` (new, diagnostic) — the bisection matrix and
  per-workload tier-stats isolation; kept as the bug's documented
  black-box repros.

## 5. The honest verdict + effort estimates

**What tier-1 buys on primitive rows today:** int-arith while-loops
~2x; everything else that reaches the twin ~1.0–1.8x depending on how
much walk overhead the primitive leaves exposed. Post-fix, every
while-loop function body in the bench corpus at least COMPILES (no more
verify-fallback), which is the precondition for any future win.

**What it would take to push the remaining FAIL rows further, ranked by
effort-per-expected-win:**

1. **For-loop lowering** (`cfg::lower_fn` `Stmt::For` → while-shape
   desugaring with iterator-protocol poll; `Break/Continue` too). This
   is THE gate: the bench harnesses (sort probes, listcomp, join/split)
   are all for-loops, so nothing else can even be measured until this
   lands. Effort: **medium (2–4 sessions)** — the while lowering exists;
   for needs the iterator protocol as boxed insts (`iter()`/`next()` via
   one helper), a StopIteration exit edge, and the same OsrPoll/eligibility
   treatment (now understood). Risk: live-set growth pushes loops over
   OSR_MAX_LIVE — raise the cap or refine liveness (locals-only is
   already the plan; the live_in forward scan is the crude part).
2. **Typed dict/getitem paths** (pon-style "typed call" region members:
   admit `Subscript` over a dict-tagged temp with a PyDict guard, call
   the compact-dict lookup inline). This is real typed-codegen work:
   guard emission, deopt on non-dict, no aliasing hazards (dict may be
   mutated by a call inside the region — only safe if the region
   contains no Call). Effort: **large (4–8 sessions)** for a dict-get
   fast path; expected win bounded by the ~985ns registry-insert cost
   which is a DIFFERENT lever (allocation, not dispatch).
3. **Listcomp specialization** (detect the comprehension shape at IR
   level, compile the loop body once, call it as a JIT'd function per
   element-batch). Effort: **large**, and the 100k-int listcomp is
   already 2100x post-Fix-C — diminishing returns.
4. **str.join/split** — these are host-native surfaces (generic
   iterable → Rust loop), NOT interpreter-walk-bound; the report's
   memchr/row-iterator levers are the right fix, not tier-1. Effort:
   n/a for this leaf (already filed elsewhere).
5. **OSR_MAX_LIVE liveness refinement** (true backward dataflow instead
   of the conservative forward scan; exclude dead locals). Cheap,
   unblocks wider OSR eligibility. Effort: **small (0.5–1 session)**;
   do it opportunistically with #1.

**Negative results (honest, valuable):**

- Tier-1 typed codegen CANNOT help dict.get/getitem rows without the
  §5.2 structural work — the twin path was already the achievable
  ceiling and it measured ~1.0x because the primitive op dominates.
- The 02.05f "Call/BinOp verify" diagnosis was wrong (it was the OSR
  live-set/emit-order bug above); the class of "silent fallback on
  specific shapes" is now explainable and has a regression test.
- One REAL silent adaptive-mode divergence existed on main (JIT'd
  ordering compares bypassing dunders) — fixed; it shows the
  transparency differential's probe set is too thin for compare
  semantics; widening it is a follow-up (one probe with a user `__lt__`
  class would have caught it; candidates listed in the s023 minimal
  repro chain).

## 6. Gates (all green, final tree)

- fix_int / fix_json / fix_sort / fix_random: byte-identical adaptive
  vs CPython 3.14.
- fix_sha_zlib: 27 divergences, ALL CORRUPT-row zlib-rs
  message-collapse family on a regenerated (not campaign-identical)
  data file — same documented upstream limitation as the report; the
  campaign's original pfz_data.bin was never committed (untracked
  scratch), so the exact 26-row divergence set is not reproducible
  byte-for-byte here; the gate property (same input → identical output
  modulo the known family) holds, and adaptive == tier-0 byte-identical.
- diff_b64 2132/0, diff_hex 1976/0.
- ap-conform values **50/50**, stdlib **48/48** (adaptive AND tier-0 —
  s023 was FAILING adaptive on main before this leaf; now green).
- cargo test -p ap-run --release: **208 passed / 0 failed / 1 ignored,
  exit 0** (includes tier_mode_differential 3/3 + osr_gate 2/2).
- cargo test -p ap-codegen-native --release: 67 passed / 0 failed.
