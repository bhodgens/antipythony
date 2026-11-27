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
