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
