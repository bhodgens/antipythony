# HANDOFF — primitive-perf + hot-path-parity + fast-path campaigns
Date: 2026-11-26. Repo: ~/git/antipythony (main @ dbec28b local; origin
main is 1 commit behind — run `git push origin main`).

## STATE
- primitive-perf + hot-path-parity: COMPLETE, all merged/pushed.
- fast-path (plans/fast-path/): 5 of 8 leaves MERGED
  (fp-bytecode e92c829, fp-frame 41bf0e0, fp-controlflow 80b221c,
  fp-calls 03571f4, fp-exceptions — verified below, MERGE PENDING).
- Closing primitive bench (pre-fast-path): 28 PASS / 36 FAIL / 0 ERROR.
- Benchmark harness: tests/bench/primitive/ (run_bench.py + 9 probes
  + 11 differential gates). Reference binary:
  target-pptrunk3/release/antipythony (MUST rebuild fresh — see GOTCHA).

## UNMERGED WORK (ready to merge)
- tier1-fp-exceptions @ 067e52b: exception-table Try/except/finally/
  raise lowering + execution in bytecode mode. VERIFIED BY ME:
  fix_bytecode 12/12 three-way GREEN (the 4 multiline-binop/unbound
  tb-line mismatches from fp-controlflow ARE CLOSED by this leaf);
  fix_int/json/sort/random + diff_b64/diff_hex GREEN on its binary;
  fix_sha_zlib GREEN modulo the DOCUMENTED zlib-rs corrupt-row message
  residual (28 rows 'repeated call with bad state' vs libz's specific
  texts; all digest/OK/TRUNC lines byte-identical; see
  fix-sha-zlib-report.md). Its own session record
  (plans/fast-path/01-core/NEXT-FP-EXCEPTIONS.md) pins 13/13 probes.
  Merge cmd: git merge --no-ff tier1-fp-exceptions.

## PENDING LEAVES (not dispatched)
- 02-perf/01-inline-caches (type_version-keyed site caches; the
  interp.rs "getattribute" case ALSO needs the native_attr fallback
  pattern this campaign proved necessary — bytes/str/int receivers
  resolve their surface through builtins_modules::native_attr).
- 02-perf/02-superinstructions (opcode-histogram-driven fusion).
- 03-rollout/01-adaptive-toggle (bytecode default flip + composition
  matrix) and 02-bench-parity (closing bench + report).

## GOTCHAS (all hit this session)
1. STALE TRUNK BINARY: .cargo/config.toml redirects builds to
   target-0304c; target/release/* is months stale; target dirs go
   stale-fingerprint after merges (cargo "Finished" without relink).
   ALWAYS: verify binary mtime, build into a FRESH CARGO_TARGET_DIR,
   and pass `--other <fresh-binary>` to fix_bytecode.py (a staleness
   guard now fails loud, commit 72eac3e).
2. fix_sha_zlib invocation: script needs pfz_data.bin beside it
   (gitignored; regen: see /tmp/pp_gen2.py pattern — seeded LCG, 1MiB)
   and antipythony does NOT forward script argv — run with NO args,
   default path, then diff CPython-leg vs antipythony-leg outputs
   yourself.
3. Sibling sessions stash/grab worktree WIP (fp-inlinecache had its
   WIP stashed by a sibling session mid-flight and recovered);
   FUZZ_SEEDS.txt goes dirty in every leaf worktree — leave it.
4. Multi-line BinOp line attribution: tree-walker and CPython report
   the LAST operand line for paren-spanned binops; fix_bytes gate
   mismatch closed by fp-exceptions (see UNMERGED).

## OPEN LEVERS (filed, with owners)
- 02-perf/01-inline-caches: also needs the native_attr-first pattern
  for native receivers in the getattribute marker arm.
- ap-gc registry batching (mint floor ~841ns — bounded the
  lazydunders/nursery targets).
- tier-1 typed dict regions + region-finder widening (for-loop typed
  regions don't form; block-0 IterNew rooting per 02.05c).
- memchr integration retry (pp2-memchr stalled pre-integration).
- sha256 backend class (OpenSSL crypto-ext vs RustCrypto).
- SyntaxWarning emitter, __class__.__name__ on builtin-raised exc,
  exec_try snapshot ordering in exc_enter, nested finally re-inline
  (fp-exceptions honest gaps 1-4).

## PERF SCOREBOARD (bench_report.json, geomean 48KiB+1MiB)
FASTER: b64decode +1963%, b64encode +1599%, b64 one-shots
+1070/+1587%, zlib.compress L6 +342%, str.find +272%, str.upper
+241%, int.to_bytes +236%, zlib.compress L1 +234%, int(str) +231%,
unhexlify +190%, int.from_bytes +101%, str.replace +34%,
zlib.decompress L1 +32% L6 +11%, hexlify +5%, json.dumps +5%.
SLOWER: dict ops -86..-99%, list.sort -86%, str.join -79%,
json.loads -60%, str.split -52%, sha256 -27%, hex(big_int) -21%,
int.to_bytes(48K shape) -22%, md5 -12%, sorted@1M -6%.


---

## ADDENDUM 2026-11-27 (post-first-handoff update)

State has advanced since this document was first written. Current truth:

### Merged to main since the first handoff
- fp-exceptions MERGED as c633a0b (branch tier1-fp-exceptions @ 97a9c49/067e52b).
  Trunk-verified first-hand: fix_bytecode 13/13 (0 mismatches), stdlib conform
  48/48, values 50/50, all 7 diff gates GREEN on target-pptrunk3 (build dated
  Nov 26 23:38). Exception table: 17 shapes lowered, tb linenos three-way exact.
- Tracker updated: 1897b4a — fast-path 01-core group COMPLETE (3/3:
  fp-controlflow, fp-calls, fp-exceptions).
- HANDOFF doc committed: e8fe3ff (this file, first version).

### In flight at stop
- deleg_15d4af97 — leaf fp-inlinecache2 (subagent sa-0-8c35506f), dispatched
  ~2h before this addendum. Scope: recover the prior inline-cache work from
  surviving branch tier1-fp-inlinecache2 (3 commits: 62118d8 caches,
  83f63bd invalidation tests + 4-thread probe, bada6b3 report — old-branch
  gates verified GREEN x3 runs + cargo 19/19), rebase onto current main
  (adapt to exception table + fused calls), complete invalidation coverage,
  no-GIL probes, gates both toggles.
  Transcript: /Users/caimlas/.hermes/cache/delegation/live/deleg_15d4af97/task-0.log
  Worktree: leaf-fp-inlinecache2 (branch tier1-fp-inlinecache2 — NOTE the
  successor must reconcile: the OLD attempt's 3 commits live on the SAME
  branch name; the new leaf rebases onto current main, expect a force-move
  or a new branch tier1-fp-inlinecache3).

### Remaining after inlinecache2 lands
1. Collect + first-hand verify deleg_15d4af97 (8 diff gates incl fix_sha_zlib
   positional-binary form, fix_bytecode 13/13, conform 50/50 + 48/48 BOTH
   toggles, ic_invalidation tests, 4-thread probe), then merge.
2. Dispatch fp-superinstructions (plans/fast-path/02-perf/02) and merge.
3. 03-rollout: fp-adaptive (bytecode default flip decision — USER GATE) and
   fp-bench (closing bench + report/chart refresh + tracker/AGENTS.md).
4. Banked user decisions: deps (crc32fast, memchr, sha2 asm-aarch64,
   bzip2/lzma-rs), rm -rf target-0304c (stale fingerprint dir), 2 unfixed
   filed bugs (loop default-arg capture, binascii except-leak).

All work is in worktrees under /Users/caimlas/.git/worktrees/; main checkout
has 41 sibling-modified files — never touched. One heavy cargo lane policy;
fresh CARGO_TARGET_DIR per leaf (stale-binary gotcha, see above).

---

## ADDENDUM 2026-11-27 14:25 MST (verification + adversarial audit)

### inlinecache2 leaf FINISHED — gates first-hand green, MERGE BLOCKED by audit
- Delegation deleg_15d4af97 completed 14:14:57. Branch `ic2-attempt` at
  a0fc09d (5 commits past main: 0492106 rebase, c8d4c78 compile fix,
  3bb8343 IC test fix, 182ab52 report, a0fc09d doc-only merge of main).
  The old branch `tier1-fp-inlinecache2` was force-moved to a0fc09d by the
  leaf; check `git rev-parse tier1-fp-inlinecache2` before merging.
- Its gates verified from its own log (clean env: `env -u APX_TIER_STATS
  -u APX_TIER`): cargo ap-bytecode+ap-run --release 21 suites, 272 passed /
  0 failed; fix_bytecode 13/13; diff_b64 2132/0; diff_hex 1976/0; diff_json
  127/0; fix_sha_zlib tw-vs-bc 0 diff (zlib-rs 28 corrupt-row residual
  unchanged); ap-conform values 50/50 + stdlib 48/48 in BOTH toggles.
  IC invalidation suite 7/7 + 3x stable green.
- FALSE RED root cause found: exported APX_TIER_STATS=1 + APX_TIER=bytecode
  in the leaf shell leaked into children, appending [BC-STATS]/[TIER-STATS]
  to stderr (import_guard_differential compares stderr LAST LINE). With
  env -u both: 9/9 green. Product gating is correct; harnesses inherit env.

### ADVERSARIAL AUDIT VERDICT: REJECT (deleg_2f2f5aa5 task 0, read-only)
- BLOCKER A `crates/ap-run/src/bytecode_interp.rs:238-263,1406-1415`:
  ic_attr_fill can record ClassValue for an instance receiver when the
  class has member x AND the instance dict has x; the hit arm serves the
  cached class value with no instance-dict re-probe (instance dict must
  win). Stamps do not bump on instance-dict writes.
- BLOCKER B `bytecode_interp.rs:1364-1405,243-245`: BindFunc hit mints a
  stale class method after `c.m = lambda` because attr_set on a user
  instance bumps no class stamp and the hit arm never re-probes o.attrs.
- HIGH `crates/ap-run/tests/ic_invalidation.rs:422`: the no-GIL gate asserts
  absence of "TEAR" but the probe emits "FREEZE" (line 376) — vacuous.
  Worker-count floor 4*10000 of 4*200000 also too weak.
- HIGH `ic_invalidation.rs:242-302`: stats lines filtered, never parsed; no
  proof the tested shape routed through bytecode/IC. A fallback would pass.
- Also: incomplete bump-point audit (classes.rs 10 direct dict.set sites),
  duplicated descriptor/bind helpers (master.md forbids semantic
  duplication), perf report numbers are pre-rebase and dict.get gain
  credited to IC despite chit=0, absolute 30ns/60ns targets NOT met.
- Orchestrator probes at the leaf binary: ic_probe_shadow.py and
  ic_probe_shadow_fn2.py returned SHADOW-X 42 / SHADOW-M instance-lambda
  on bytecode, tree0 and CPython 3.14 — but ahit=0, so the audit BLOCKERs
  are NOT yet reproduced on a genuine IC hit. A repair leaf must prove
  ahit>0 before patching.

### Dispatched repair wave (deleg_8376bec8, both background)
- Leaf A (sa-0-e787e037) worktree `leaf-fp-inlinecache`, branch ic2-attempt
  @ a0fc09d: prove or refute the two BLOCKERs with an ahit>0 probe; if
  reproduced, re-probe o.attrs on ClassValue/BindFunc hits; fix FREEZE
  assertion + add two shadowing regression gates; re-run 21 suites,
  fix_bytecode 13/13, conform 50/50 + 48/48; commit on ic2-attempt.
- Leaf B (sa-1-10102d2a) worktree `leaf-fp-gate-env`, branch fp-gate-env off
  main: shared APX_* diagnostic env sanitizer + strip_diagnostic_lines for
  ap-run/ap-cli differential harnesses; verify under simulated env leak.

### Other landed work
- `fp-rollout-matrix-design` @ bb2731e (worktree leaf-fp-rollout):
  docs/fast-path-rollout-matrix-design.md, 159 lines. Key finding: the
  plan's "bytecode fn → tier-1 fn → bytecode fn" composition is UNREACHABLE
  today — tier mode is one thread-local and bytecode dispatch is checked
  before JIT; `jit` is an alias of `adaptive`. The missing per-function
  dispatch seam is a prerequisite for rollout step 1.

### Do NOT merge until
1. Leaf A reports; BLOCKERs reproduced-and-fixed (or disproved with ahit>0
   evidence) and its gates re-run green on its own branch.
2. Leaf B reports; decide whether gate sanitizer merges before or after
   ic2-attempt (both touch test harnesses — check overlap first).
3. Re-verify first-hand on a FRESH CARGO_TARGET_DIR; then merge to main.
4. Dispatch fp-superinstructions (worktree leaf-fp-superinst, branch
   tier1-fp-superinst-prep, created at 2c2648b) only after the merge.

Disk at 14:01: 51 GB free / 927 GB (95%). One heavy cargo lane only.

---

## ADDENDUM 2026-11-27 15:05 MST (repairs merged, trunk verified)

### Merges on main (current HEAD fe7cc86)
1. 2d5ad8f — merge fp-gate-env (a9f58c6): shared APX_* diagnostic env
   sanitizer in ap-run/ap-cli differential harnesses + clean-env docs.
   Verified by leaf: import_guard 9/9 under INJECTED env leak and clean,
   tier_mode 3/3, risk4 3/3, import_differential 6/6, osr_gate 2/2.
2. 606926b — merge fp-inlinecache2 (2f28b4a). Both adversarial BLOCKERs
   REPRODUCED on genuine IC hits (ahit=3000, afill=1): warm c.x==1 then
   c.x=42 returned stale 1; warm c.m() then c.m=lambda returned stale
   method. Fix re-probes the live instance dict on ClassValue/BindFunc
   hits and clears the entry when shadowed. Two regression tests added;
   no-GIL gate now asserts FREEZE (was TEAR), strict per-worker counts,
   sustained concurrent mutation. ic_invalidation 9/9.
3. fe7cc86 — ic_invalidation.rs hardcoded target-fp-ic2b; changed to
   resolve the binary through CARGO_TARGET_DIR (fallback target-0304c).
   Found by first-hand trunk verification, not by the leaf.

### First-hand trunk verification (fresh target-pp-fastpath, clean env)
- cargo build --release -p ap-cli -p ap-conform: OK (3m24s); binary mtime
  14:50, fresh. Debug ap-run-eval also built (fix_pass gate resolves
  CARGO_TARGET_DIR/debug/ap-run-eval).
- 21 suites / 274 passed / 0 failed / 1 ignored (ap-bytecode + ap-run
  --release). Logs: ~/.git/worktrees/fastpath-cargo-suites3.log.
- ic_invalidation 9/9. fix_bytecode.py --other target-pp-fastpath/release/
  antipythony: 13/13, 0 mismatches. ap-conform values 50/50, stdlib 48/48
  in BOTH tree0 and bytecode modes.

### Dispatched
- deleg_6a71243e — fp-superinstructions (tier1-fp-superinst @ fe7cc86,
  worktree leaf-fp-superinst). PROFILE FIRST with APX_BP_STATS=1 histogram;
  data-driven fusions only; report committed. 02-perf closes with this leaf.

### Next after superinst lands
- 03-rollout/01-adaptive-toggle: USER GATE for default flip. Prerequisite
  found by bb2731e: the per-function tier dispatch seam does not exist
  (one thread-local mode; bytecode arm precedes JIT); implement it first.
- 03-rollout/02-bench-parity: closing 64-row bench + report + chart.
- Disk 34 GB free / 927 GB (97%) at 15:00 — one target dir per leaf, no
  new heavy builds without checking df.

---

## ADDENDUM 2026-11-27 16:05 MST (02-perf complete)

### Merged
- 404dc98 — merge fp-superinstructions (ea23b54 profile infra +
  7391908 fusions). Main also contains sibling merge 3141515 (wasm
  zero-arg super(), 25 files, NO overlap with the superinst leaf).
- 3 histogram-backed fusions: LoadConst2BinOp (9.24M triple, 1.12x),
  LoadFastConstBinOp (2.35M, 1.15x), LoadFast2BinOp (0.42M, 1.14-1.16x).
  Guarded Smi fast path; generic eval_binop fallback. 5 new ap-bytecode
  unit tests. Raw histogram + report committed. Dropped 4 candidates with
  no measured win; primitive-bench rows honestly labeled NOISE
  (module-level loops never dispatch bytecode).

### First-hand trunk verification (fresh target-pp-fastpath)
- cargo build --release -p ap-cli -p ap-conform: OK (2m55s), binary
  mtime 15:54. fix_bytecode 13/13. ap-conform 50/50 + 48/48 in BOTH
  tiers.
- Full ap-bytecode+ap-run --release suite INTERRUPTED at ~7 min by the
  420s terminal cap under load 31-53 (sibling session runs
  `cargo test --workspace` in verify-merge-super at HEAD 404dc98).
  Leaf measured 21 suites / 279 passed / 0 failed / 1 ignored; sibling
  workspace run had 0 failures through 13 suites at last check.
  Re-run the scoped suite in a quiet window before the rollout gate.

### Next
- 02-perf COMPLETE. 03-rollout is USER GATED (default flip). Prerequisite:
  per-function tier dispatch seam (bb2731e design doc). Disk 34 GB free.

---

## ADDENDUM 2026-11-27 17:30 MST (tier seam merged)

### Merged
- 506b976 — merge fp-tier-dispatch-seam (ee6ddae). Explicit per-function
  tier override via APX_TIER_SEAM_OVERRIDE / --tier-seam
  'bytecode:outer,adaptive:mid,bytecode:inner', keyed by FnEntry Arc
  pointer (process-global OnceLock, no source markers). Bad specs fail
  loudly. DEFAULT UNCHANGED — no flip.
- Mixed chain PROVEN first-hand on fresh target-pp-fastpath binary:
  [TIER-SEAM-STATS] bytecode_execs=62 adaptive_execs=31 tree_execs=0,
  [TIERUP-STATS] tier1=1, [BC-STATS] execs=62, stdout 1999003 =
  CPython 3.14 output byte-for-byte.
- tier_seam_differential 3/3 green on trunk. Leaf reported ap-run 20
  suites / 225 passed / 0 failed / 1 ignored.

### Verification GAP (honest)
- My own full ap-run --release suite run was BLOCKED by an approval
  timeout on the terminal command; I did not retry it. Before the seam
  merge I had verified 21 suites / 274 passed / 0 failed on trunk.
  The seam leaf reports its own full suite green. Re-run in a quiet
  window before the final gate sign-off.

### Next (03-rollout, no flip yet)
- Run the composition matrix (20 meaningful cells) and the standard
  battery, then the candidate-mode (bytecode) closing 64-row bench.
  Default flip remains the sole USER GATE.

---

## ADDENDUM 2026-11-27 17:50 MST (closing bench landed — GATE RED)

### Merged
- Closing bench leaf 5ac23b0 merged (merge commit follows). 7 owned files:
  64-row bytecode-mode bench_report.json (generated 2026-11-28 00:35 UTC),
  BOTH charts regenerated from that JSON, docs/fast-path-report.md (277
  lines), README perf section, benefits doc sync, NEXT-FP-BENCH.md.
- Verified first-hand: 64 rows / 28 PASS / 36 FAIL / 0 ERROR; geomean
  1.180x; simple mean 3.292x; all 64 ratio labels present in regenerated
  speedup SVG (computed, not transcribed).

### EXIT GATE: RED (honest result, no gate weakened)
- ALL 18 walk-bound rows below 1.0x:
  dict.getitem 0.205/0.267/0.268x, dict.build 0.181/0.192/0.192x,
  list.sort 0.168/0.112/0.146x, sorted() 0.828/0.849/0.811x,
  str.join 0.193/0.230x, str.split 0.451/0.509x,
  json.loads 0.509/0.289x.
- Simple-mean floor >=1.5x PASSES (3.292x) but the parity clause fails.
- ROOT CAUSE: probe hot loops are MODULE-LEVEL and bypass call_function
  tier dispatch. BC-STATS on P09 = compiles=6 fallbacks=3 execs=79,
  TIER-STATS execs=0. Function-shaped helpers DID improve: dict.get
  0.01x -> 0.24-0.28x, sorted 0.4x -> 0.81-0.85x. list.sort/join/split/
  json.loads remain sub-1.0x even when function-shaped.

### Gates all GREEN on leaf binary (SHA256 068cc7f2...160)
fix_bytecode 13/13; diff_b64 2132/0; diff_hex 1976/0; fix_int 18653
lines byte-identical; fix_json 127/0; fix_sort 42/0; fix_random PASS;
fix_sha_zlib 818 lines 0 diff tree-vs-bytecode (62 CORRUPT-only zlib-rs
residual vs CPython); conform 50/50 + 48/48 BOTH tiers; tier_seam 3/3;
ic_invalidation 9/9; cargo ap-bytecode+ap-run --release rc=0,
22 result blocks, 282 passed / 0 failed / 1 ignored.

### DECISION STATE
- Default flip NOT executed (USER GATE untouched).
- Per master.md exit gate, fast-path campaign does NOT exit green.
  Recommend a new leaf: function-shape the 18 walk-bound probes (or
  lower module-level loops) before re-running; otherwise file the gate
  as an honest gap and let the user choose.
- Disk 30 GB free / 927 GB (97%) at 17:45.
