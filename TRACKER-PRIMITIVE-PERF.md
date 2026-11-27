# TRACKER-PRIMITIVE-PERF — campaign tracker (orchestrator session 2026-11-24)

Plan tree: `plans/primitive-perf/` (master.md is the contract).
Worktree: `/Users/caimlas/.git/worktrees/leaf-primperf` (branch `tier1-primperf`,
branched from main @ b1c89a1). ALL leaf work happens there. Merges go back to
main via the orchestrator after first-hand gate re-runs.

## Machine / environment pins

- Mac M2 Ultra, aarch64, NEON. ONE heavy cargo lane; CARGO_BUILD_JOBS=4.
- Leaf target dir: `target-primperf` inside the worktree.
- CPython reference: /opt/homebrew/bin/python3.14 (version-string checked).
- antipythony release binary: `~/.git/worktrees/leaf-primperf/target-primperf/release/antipythony`
- Disk guard fired at session start: 28G free -> cargo clean on merged leaf
  worktrees reclaimed ~28G (56G free at dispatch). Do not re-clean without
  re-measuring.

## Frozen contracts

- bench_report.json schema: plans/primitive-perf/00-harness/orchestrator.md
  ("Interface contract (frozen)"). Single-writer rule: only ONE agent at a
  time ever refreshes bench_report.json (orchestrator serializes).
- File ownership: 01-encode-simd group owns
  crates/ap-run/src/builtins_modules.rs (b64/hexlify arms only) + new
  crates/ap-run/src/simd_encode.rs. 02-parity-audit leaves are AUDIT-ONLY
  (write only plans/primitive-perf/02-parity-audit/audits/*.md). Fix leaves
  from audit findings are spun out AFTER 01-encode-simd frees the file.

## In-flight / queue

| # | leaf | owner surface | status | delegation |
|---|---|---|---|---|
| W1 | 00-harness/01-harness-runner.md | tests/bench/primitive/* (new files, no cargo) | DONE — orchestrator-verified (schema OK, 6 smoke rows, reproducible) | deleg_7a6d3a8f |
| W1 | diff gates diff_b64.py + diff_hex.py (upfront gate authoring) | tests/bench/primitive/diff_*.py only | DONE — verified (hex 1832/1832 GREEN; b64 RED w/ 8 scoped API divergences, routed into base64 leaf brief) | deleg_1f905825 |
| W1 | audits: hash-compress / str-bytes / sort-dict-json (analysis-only, ratio placeholders) | 02-parity-audit/audits/*.md ONLY | DONE — verified (764 lines total; found 2 to_bytes/from_bytes correctness bugs, 3 sort correctness bugs, P10 baseline discrepancy which orchestrator reconciled) | deleg_43751b52 |
| W1 | release build of ap-cli in leaf worktree | target-primperf | DONE (ap-cli + ap-conform release, smoke-verified) | proc_196de983a269 |
| W2 | 00-harness/02-baseline-survey.md | bench_report.json + baseline-notes.md | DONE — orchestrator-verified (50 rows, 17P/25F/8E, ranking consistent, cites read) | deleg_6a6a9f3c |
| W3 | 01-encode-simd/01-base64-simd.md | builtins_modules.rs b64 arms + simd_encode.rs | MERGED to main (dd3eda0) — gates re-verified on trunk: diff_b64 2132/2132, diff_hex 1832/1832, values 50/50, stdlib 48/48, cargo release green. encode 16.8-18.7 GB/s (target 24 = honest gap: dispatch overhead + M2-vs-M5 hardware), decode 10.0-10.9 GB/s (target 14, gate >=10 MET) | deleg_58d14553 |
| W4 | 01-encode-simd/02-hexlify-simd.md | binascii hex arms + simd_encode.rs | MERGED to main (a0dc0f8) — gates re-verified on fresh trunk binary: diff_hex 1976/1976, diff_b64 2132/2132. hexlify 2.06 GB/s PASS (1.23x cpy), unhexlify 6.53 GB/s PASS (3.81x cpy); 10x aspiration = interpreter-arm ceiling honest gap | deleg_8e87fec8 |
| W3 | 02-parity-audit/01-hash-and-compress.md | audits/hash-compress.md ONLY | DONE — merged (3cb3a55); headline fixes: sha2 asm-aarch64 flag (existing dep, est 4-8x), crc32fast (in lock, est 10-30x); bz2/lzma = NotImplementedError shims (new-dep user decisions) | deleg_43751b52 |
| W3 | 02-parity-audit/02-str-and-bytes.md | audits/str-bytes.md ONLY | DONE — merged (3cb3a55); 2 correctness bugs (to_bytes zero-fill, from_bytes truncation >16B), int(str) no small-int fast path, memchr = user dep decision | deleg_43751b52 |
| W3 | 02-parity-audit/03-sort-dict-json.md | audits/sort-dict-json.md ONLY | DONE — merged (3cb3a55); json is NATIVE (dumps 0.97x, loads 0.17-0.24x w/ opt leaf sketch); dict ALREADY-PARITY; 3 sort correctness bugs; int-loop pathology ~35us/elem filed (explains P09 timeouts) | deleg_43751b52 |
| W5 | 99-report/01-final-report.md | docs/primitive-perf-report.md + chart.svg | MERGED to main (1e25df5; branch commits 9a47546/72e912d) — chart==JSON verified; orchestrator corrected attribution: the 8 audit-found bugs are FILED-not-fixed (verified reproducing on trunk) | deleg_838ea88d |
| W6 | ppfix-perf: sha2 asm-aarch64 + zlib one-shot decompress | ap-run Cargo.toml features + zlib decomp/hashlib arms | MERGED to main (af172e8) — trunk gates: fix_sha_zlib green, values 50/50 + stdlib 48/48, cargo 18/18. zlib.decompress 0.38-0.52x -> 1.02-1.33x PASS x4; sha256 0.14x -> 0.88x (~6x; residual vs OpenSSL crypto-ext = backend-class gap, new-dep decision if we want the last 12%) | deleg_ac90809e |
| W6 | ppfix-int: to_bytes/from_bytes bugs + hex limb path | builtins_modules.rs int arms + builtins.rs hex/oct/bin | MERGED to main (6bb2ab9) — trunk-verified: fix_int green, both bugs closed first-hand, values 50/50 + stdlib 48/48 | deleg_ac90809e |
| W6 | ppfix-sort: 3 mixed-type/key=None sort bugs | builtins.rs sorted + methods.rs/builtins_modules.rs sort arms | MERGED to main (b6afd8c) — trunk-verified: fix_sort 42/42, sorted-mixed now raises exact TypeError, values 50/50 + stdlib 48/48; bisect: Int pathology = superlinear listcomp/append, NOT compare dispatch | deleg_ac90809e |
| W6 | ppfix-random: implement random.Random class | native random module surface | MERGED to main (b23dfd1) — trunk gates: fix_random pass, values 50/50 + stdlib 48/48, cargo 18/18, p08 probe rc=0 emitting rows. MT19937 CPython-exact sequences; 2 pre-existing PRNG divergences fixed en route. Honest gaps: seed(float/str/bytes) TypeError, negative-step randrange, getrandbits k>64 clamp, repr no-address | deleg_ac90809e |
| W7 | ppfix-fixc: Int loop pathology + container-mint floor (STRUCTURAL) | ap-obj/*, ap-gc/*, ap-run/interp.rs listcomp/append arms | MERGED to main (1ad9559) — trunk-verified 7/7 gates + values 50/50 + stdlib 48/48 + cargo 18/18. Roots: O(n^2) items() per iter pull; gen-1 GC retain DEREGISTERED live old objects (real fence bug); PyObject dunders double-alloc. int loop 57100->190ns/elem (300x), 100k listcomp 17.6s->8.2ms (2100x), dict build 3106x->11x, getitem 2027x->13.9x, d.get 2104x->111x. Honest: loads 0.22->0.33x (registry insert ~985ns; next = batching/nursery); walk floor 190ns/step = tier-0 itself. Follow-up hotfix on trunk: ppfix-int's from_bytes per-chunk shift-accum was O(n^2) — bulk Big::from_bytes_be (256e5d9), 1MiB 2.63s->1ms, P07 from_bytes back to 1.83-2.45x PASS | deleg_fd6dd232 |
| W8 | pp2-int: to_bytes bulk emit + small-int parse fast path + D&C big parse | ap-obj/src/int.rs | IN FLIGHT (worktree leaf-pp2-int) — deps pre-approved: none needed | deleg_b044bfe9 |
| W8 | pp2-sort: Fix B special_lookup hoist + homogeneous sorted() raw arm | builtins.rs sorted + sort arms | IN FLIGHT (worktree leaf-pp2-sort) | deleg_b044bfe9 |
| W8 | pp2-memchr: memchr dep (USER APPROVED) — bytes find/replace/split/count | ap-run Cargo.toml + bytes arms | IN FLIGHT (worktree leaf-pp2-memchr) | deleg_b044bfe9 |
| W8 | pp2-crc: crc32fast + simd-adler32 (USER APPROVED) — kill scalar CRC loops | ap-run Cargo.toml + crc/adler arms | IN FLIGHT (worktree leaf-pp2-crc) | deleg_b044bfe9 |
| W8 | pp2-bz2: real bz2 + lzma modules (USER APPROVED deps; pure-Rust preference) | vendored shims -> native surfaces + ap-run arms | IN FLIGHT (worktree leaf-pp2-bz2) | deleg_b044bfe9 |
| W8 | pp2-tier1: tier-1 dispatch research + cheapest bridge prototype | docs/tier1-primitive-dispatch.md + probes | MERGED to main (55982d3, rebased onto current main first) — trunk gates: 7/7 diff green, values 50/50 + stdlib 48/48 in BOTH tier-0 and adaptive modes, ap-run 18/18 + ap-codegen-native 10/10 cargo green. Landed: StoreSubscript bridge (dict_build twin 1.82x), OSR-poll eligibility-first fix (live-set>16 was silently falling back to tree-walk — 02.05h misattribution corrected), COMPARE_FULL_FN rich-compare seam (fixes pre-existing s023 adaptive divergence). Deltas: int-arith 2.05-2.09x, str_concat 1.64-1.76x, str_find 1.07-1.14x. Honest: for-loop bodies still lower-reject (Stmt::For Unsupported); typed regions decline dict/str loops by design; walk floor 190ns/step remains tier-0's cap on non-JIT'd rows | deleg_42bcc34d |
| W9 | hp-nursery: GC bump arena + batch registration (985ns -> <=100ns/mint) | ap-gc/* + gcsupport + dict/list new_in | IN FLIGHT (worktree leaf-hp-nursery) — plan plans/hot-path-parity/ | deleg_6f39a992 |
| W9 | hp-lazydunders: lazy dunders/attrs + bound-method fast path (1.5µs -> <=300ns) | ap-obj object.rs/value.rs + methods.rs | IN FLIGHT (worktree leaf-hp-lazydunders) | deleg_6f39a992 |
| W9 | hp-forloop: Stmt::For JIT lowering + backward-dataflow liveness (THE gate for sort/listcomp/join shapes) | ap-codegen-native/* | IN FLIGHT (worktree leaf-hp-forloop) | deleg_6f39a992 |
| W9 | hp-inlinecache: per-site resolution cache | methods.rs/interp.rs + new inline_cache.rs | QUEUED (after nursery merges) | - |
| W9 | hp-typeddict: typed dict getitem/setitem codegen (LARGE) | ap-codegen-native/* | GATED (after nursery+forloop merge + re-measure) | - |
| W9 | hp-micro: BE single-pass to_bytes + str.split/join fast paths | int.rs BE + split/join arms | QUEUED (check pp2-int/pp2-memchr merged state first) | - |
| W6b | ppfix-json: json.loads/dumps bulk scan + escape tables | json region of builtins_modules.rs (~:7361-7843, region-disjoint from W6) | MERGED to main (55adf5a) — trunk-verified: fix_json 127/127, dumps 1.02x/1.13x PASS both sizes (0.92x gap closed), loads kernels 1.4-2.2x faster than CPython (residual = container-mint floor, wave-2); values 50/50 + stdlib 48/48 | deleg_cd8e6817 |
| W6b | ppfix-dictprobe: p09 probe restructure + per-op dict ratios | tests/bench/primitive/probes/p09_dict.py + report ONLY | MERGED to main (2b205cd) — 6 P09 ERROR rows unblocked (12 real rows, zero ERROR); KEY FINDING: Int interpreter-loop pathology is ~97% of every dict row (loop alone ~4500x CPython, d[k] only 13.5x, d.get bound-method dispatch +5.2us/call); wave-2 target = Fix C (Int loop), NOT dict structure (ALREADY-PARITY confirmed) | deleg_cd8e6817 |

## Dispatch waves

- W1: harness leaf (Python-only) + orchestrator-side release build, parallel.
- W2: baseline survey (single agent; owns the bench_report.json writer role).
- W3: base64-simd leaf + all 3 audit leaves in parallel (file-disjoint:
  base64 owns builtins_modules.rs b64 arms; audits write only .md docs).
- W4: hexlify-simd after base64 is REVIEWED (shares simd_encode.rs).
- W5: final report after W3+W4 merged to trunk and gates re-run.
- Audit leaves may run ahead of W2: they write analysis with
  'PENDING baseline (W2)' ratio placeholders; orchestrator fills measured
  numbers at review. Audit leaves write ONLY their audits/*.md file.
- Gate-authoring leaf (diff_b64/diff_hex) runs ahead of the SIMD leaves
  so they inherit a validated RED→GREEN differential gate.
- Audit FIX leaves (if any FIXABLE verdicts): dispatched by master after
  01-encode-simd file ownership frees up (i.e. after W4 merges).

## Merge gates (verify-before-merge, every implementation leaf)

- Build the full gate dependency set before running gates:
  `cargo build --release -p ap-cli -p ap-host -p ap-aot-crt
  -p ap-aot-crt-hybrid` + debug probe bins in the leaf's target dir.
  The hybrid gate needs ap-host + BOTH CRT staticlibs; CI never built
  the release probe, so a leaf that skips this reports false reds.

- Differential byte-exactness gate FIRST (leaf's diff_*.py), then perf.
- ap-conform values + base64/hex stdlib corpus slices green.
- `cargo test -p ap-run --release` green in the leaf's target dir.
- Orchestrator re-runs headline gates first-hand in the worktree before
  merge; re-runs on trunk after merge (fresh binary rebuild).
- Never weaken a corpus case. No new crate deps without user sign-off
  (memchr/serde_json proposals from audit leaves = user decision items).

## Completion definition (exit gate)

docs/primitive-perf-report.md exists with the chart; every P-row in
bench_report.json verdict PASS (antipythony >= upstream SOTA target) or
carries a documented deferral with the measured gap. Final full bench
re-run on merged trunk produces the report numbers. AGENTS.md status
block + this tracker updated at close-out.

## NOT in scope

- JIT/tier-1 performance (plan measures tier-0 native builtin speed by design).
- Wasm leg of the corpus gates (native-only perf campaign; wasm rows unchanged).
- Vendoring serde_json/memchr without explicit user approval.

## W9 hot-path-parity (plan: plans/hot-path-parity/) — COMPLETE 2026-11-26

| leaf | surface | status |
|---|---|---|
| hp-nursery | ap-gc ledger + dying-young flush + 3 re-entrancy/deadlock fixes | MERGED (3d3cd61) — final report: mint 1119-1193ns -> 756ns die-young (-23%); <=100ns target NOT met (fallback promote-at-flush shape; remaining = subtract-refs walk ~25ns/obj + 3-map critical section ~200ns, not batching-removable); json.loads now parity (32.5 vs 31.1 MB/s); 3 REAL bugs fixed en route incl. mid-collect ladder self-deadlock (gc014 hang, sample-proven) |
| hp-lazydunders | unregistered attrs payload + bound-method mint fast path | MERGED (2b19e4c) — final report: method mint 3488 -> 2400ns (raw floor ~841ns; residual = Gc alloc + 15 locks); repeat attr read ~558ns flat (MRO walk — type_version cache filed); 2 byte-exactness regressions from fast-path attempts caught by gates and fixed |
| hp-forloop | Stmt::For JIT lowering (5 iterator kinds) + OSR header + liveness | MERGED (f8352db) — int-accum 2.16x; 4 JIT boundary bugs fixed (OSR header re-ran iter() doubling subtrees, JIT'd raise -> SystemError, metaclass classmethod raw descriptor, NotImplemented is-check inverted). Honest: typed regions still don't form for for-loop bodies (block-0 IterNew rooting) — region-finder widening is the follow-up |
| pp2-int | to_bytes bulk emit + small-int parse + D&C big parse | MERGED (97d2e45) |
| pp2-sort | Fix B hoist + homogeneous raw arms | MERGED (871be30) |
| pp2-memchr | SALVAGE: fix_bytes gate only (integration not landed) | MERGED (4621d5c) |
| pp2-crc | crc32fast + simd-adler32 | MERGED (a71b6c2) |
| pp2-bz2 | real bz2 + lzma (pure-Rust) | MERGED (8333828) |
| pp2-tier1 | JIT bridges + verdict doc | MERGED (55982d3) |
| orchestrator hotfix | int-needle find/count/replace/split/startswith + start/end clamp + empty-needle + __getattribute__ marker | MERGED (a3f53fd) |

Closing bench (64799d4): 64 rows = 28 PASS / 36 FAIL / 0 ERROR.
New PASS: int.to_bytes 2.97-3.79x, int(str) big 4.31x, json.loads 48KiB
improved to 0.55x. Remaining FAILs documented in honest-gaps; largest
levers filed: registry-insert batching/nursery, BoundMethod mint,
tier-0 walk floor (tier-1 = Plan 02.05), str.join/split, sha256/md5
backend class. Queued not-dispatched: hp-inlinecache, hp-typeddict
(gated on re-measure), hp-micro.

## FAST-PATH plan (plans/fast-path/) — bytecode lowering to kill the walk floor

New hierarchical plan created 2026-11-26 (commit 08b8bed). 8 leaves in
4 groups: 00-infra (ap-bytecode crate + dispatch interpreter),
01-core (control flow, calls/attrs, exceptions), 02-perf (inline
caches, superinstructions), 03-rollout (adaptive default + closing
bench). Target: walk floor 190ns/step -> ~10-15ns dispatch; the
walk-bound rows (dict/sort/json.loads/join/split) to parity-or-better.
Byte-exactness contract unchanged; tree-walker stays as fallback.

Status 2026-11-26: fp-bytecode MERGED (e92c829 — 31 opcodes, lowerer,
32 unit tests, function-granularity FallbackToTreeWalk; pure addition).
fp-frame MERGED (41bf0e0 — BytecodeInterp 927-line dispatch loop,
thread-local line bookkeeping, --tier bytecode toggle, fix_bytecode
three-way gate 8/8, both conform slices green both toggles, 242 cargo
tests green). Gate caught 2 bugs: unbound-vs-None locals sentinel,
StoreSubscript operand scramble. 01-core control-flow MERGED (80b221c): While/For lowering + ForNext
+ fused LoadAttrCall + smi dispatch + gate loop coverage. Numbers:
1M loop build 4.4x vs tree-walk (target 5x missed — LCG bigint ops,
ap-obj bigint fast path filed), dispatch-only 19.7x, while-counter
3.04x. GATE FINDING (orchestrator-verified, kept open): multiline-
binop traceback lines attribute to the statement line in bytecode
mode (bc=54 vs py/tw=55 on the gate probe; repro /tmp/pp_s_paren.py).
Values identical everywhere — attribution only. OWNED by 01-core/03
exceptions (traceback line attribution is its spec). fix_bytecode
also gained a staleness guard on default_binary (72eac3e — stale
trunk binary was silently failing the exercise probe).
fp-calls-and-attrs IN FLIGHT (Call/LoadAttr/Subscript opcodes +
comprehension lowering; targets dict/str method-call loops >=2x vs
tree-walk). Then fp-exceptions (owns the tb-line fix).
Tree-walker default unchanged until parity proven.

fp-calls MERGED (03571f4 — Call/LoadAttr/StoreAttr/Subscript opcodes +
comprehension lowering; str method-call loops 8.93x vs tree-walk;
fix_bytecode extended to 12 cases, green; known gaps: nested-loop
ForNext handle-cache thrash + kwargs binder + user-instance attr walk
all owned by 02-perf). fp-exceptions IN FLIGHT (exception-table
lowering + execution; ALSO owns the multiline-binop tb-line gate fix
carried from fp-controlflow — 4 mismatches to close). 01-core at 3/3
after fp-exceptions lands.
