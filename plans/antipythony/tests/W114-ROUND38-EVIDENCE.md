# W114 round-38 evidence + memory-safety constraints (2026-11-05)

## Leak attribution (from sibling agent's evidence pack, acted on)
- Repro: /tmp/probe_unittest.py (unittest one-test runner).
- HEAD tree: 7.3s / 67 MB. WIP tree (sibling uncommitted crates/ap-run/src/
  edits): multi-GB growth under every GC policy.
- Sawtooth RSS profile (43 GB -> 16 GB within one run): GC passes reclaim
  aggressively; the path's problem is ALLOCATION RATE between passes, not
  reclamation failure.
- Resolution: sibling leaf landed bcc0a04 (perf(gc): generational cascade
  700/10/10 ladder + scoped scans, kills O(n^2) auto-collect; perf(parse):
  one-shot line index). This matches the sawtooth diagnosis.
- HARD RULE from user: NEVER launch antipythony probes that can balloon
  memory again — the machine does not have it. Any run must go through the
  dea6c03 RSS/deadline-fenced executor (subprocess-per-probe harness), or
  carry an explicit `ulimit -v` / RSS fence. No bare
  `./target/release/antipythony run` invocations on untested trees.

## My round-38 files (uncommitted, verified leak-free by attribution)
- vendored_stdlib/ast.py: bytes-decode in parse() + dump=_dump_impl alias
  (closure-poisoning workaround; commit 796195b documents the compiler bug).
- vendored_stdlib/_ast.py + crates/ap-run/src/ast_bridge.rs: is_async as
  int, real bytes Constants, AsyncWith/AsyncFor variants, bigint literals
  via parse_base10, posonlyargs split.
- Note: ast.py carries TWO duplicate bytes-decode blocks (lines ~78-88);
  collapse to one before committing.

## Round-38 verification (blocked, then resumed)
- Script: /tmp/w114_verify.py — diffs functools.py/contextlib.py ast.dump()
  output vs CPython golden /tmp/w112_recheck_cpy.json.
- Blocker was the leak (sibling WIP); resume ONLY after rebuilding on a
  tree containing bcc0a04 AND behind the RSS fence.
- Open: functools/contextlib dump_one still unverified vs golden; earlier
  attempt pre-leak showed them passing through the alias path cleanly.
