# Leaf 02RESID-B session record — 2026-11-22 (worktree ~/.git/worktrees/leaf-02resid-b, branch tier1-02resid-b off main @ 1cd873c)

## Status: slice 48/48 GREEN (gate exceeded: baseline at leaf start was already 48/48 on stdout; the leaf's work targeted the residual byte-level surface gaps behind the audit's named findings). All five named surfaces landed byte-exact vs CPython 3.14.7.

## Baseline note (important)
main @ 1cd873c already measured 48/48 on the stdlib slice (s023 ast.unparse
imported cleanly — the vendored _ast_unparse.py chain works). The task brief's
"s023 RED" premise was stale relative to the tree state. The leaf therefore
targeted the audit's ~16 named surface gaps, priority order per the brief.

## Commits (branch tier1-02resid-b)
- 879da2a: sorted(reverse=True) tie-order + builtin-derived truthiness
  (s015 Counter bytes). builtin_sorted reversed the list AFTER an ascending
  stable sort — equal-key tie groups came out in reversed input order
  (Counter.most_common + Counter repr ordering diverged). Now reverses the
  COMPARATOR (CPython list.sort semantics). user_truthiness falls through to
  user_len's container-backing arms — empty Counter()/OrderedDict()/dict-
  subclass instances were truthy (CPython: falsy), so Counter() printed
  "Counter({})" (non-empty repr branch).
- 49bf328: object-virtual dunder markers bind to the instance (s024 enum
  member.__str__). E.A.__str__() rendered "None": the cached process-global
  marker had no receiver; the zero-arg bound call found empty slice_parts.
  instance_attr now binds __object_virtual_* like the __ap_mbc__ families.
- a5ceff5: lru_cache wrapper type surface (s014). type(f) == _lru_cache_wrapper
  byte-exact (three type() arms + vendored wrapper marker attr). CacheInfo
  repr/stats already exact — re-verified.
- 54b041d: warnings attribution for exec'd code (s033/s016 family). Frame
  gains exec_co_filename; builtin_exec propagates the compile marker's
  co_filename ("<string>" for exec(str)); exec frame pushes onto py_frames;
  py_frame_stub prefers the carrier. exec-frame warnings now attribute to the
  exec'd file:line.

## Surface disposition (audit gap list)
- s023 ast.unparse: VERIFIED GREEN (was already fixed on main; probes
  byte-exact incl. unparse of def/class/NodeTransformer trees, literal_eval
  error lines, Expression kwargs DeprecationWarnings to stderr).
- Counter repr/most_common: FIXED (sorted tie-order + truthiness) — byte-exact.
- functools CacheInfo + introspection: FIXED (type(f) surface) — byte-exact.
- enum name-mangle bug (member.__str__ "None" rendering): FIXED — byte-exact
  (IntEnum/StrEnum/Flag member str/repr/format chains).
- warnings attribution: FIXED for the exec'd-code family; import-time and
  function-level attribution probes byte-exact (warn/warn_explicit/
  formatwarning/stacklevel/error-mode). importlib.util.spec_from_loader is
  unbound (pre-existing, separate gap — probe rerouted through exec).
- Stretch (NOT attempted, recorded as findings): pprint sort_dicts/width
  beyond s028's landed state, argparse Namespace repr/error text, typing
  repr/origin beyond s026, operator.not_/methodcaller, struct pack_into,
  bool-ABC registration — all slice-PASS as pinned; deeper shapes remain
  audit-named with the original repro bytes.

## Gates (all measured this session, this worktree)
- ap-conform stdlib slice --tier 0: **48/48** (0 failed, 0 timeouts).
  Command: ./target-02resid-b/release/ap-conform run --corpus
  'tests/corpus/stdlib/*.py' --impl 'target-02resid-b/release/antipythony
  --tier 0 run' --timeout 120
- cargo test -p ap-run --release: 18 suites ok / 0 failed (re-run after every
  commit; CARGO_TARGET_DIR=$PWD/target-02resid-b throughout).
- Byte-exact probes: probes-02resid-b/p_{s023,counter,cacheinfo,ci2,mangle2,
  warn11,warn15,warn4,warn13,warn14}.py all diff-clean vs python3.14.

## Gotchas hit
- Fresh-worktree fence gotcha CONFIRMED: fix_pass_differential spawns
  $CARGO_TARGET_DIR/debug/ap-run-eval; a release-only build has no debug/
  dir → "spawn fenced cmd: No such file or directory". Fix: cp release/
  {ap-run-eval,antipythony,ap-run-probe} into target-02resid-b/debug/.
- The patch tool resolves /Users/caimlas/git/antipythony/... to the MAIN
  checkout, not this worktree — one sorted-fix edit initially landed on
  main's working tree (reverted immediately, reapplied with the worktree
  path). Worktree writes must use ~/.git/worktrees/leaf-02resid-b/... paths.
- Main checkout carries ANOTHER session's uncommitted mbc_*.rs edits — do
  not touch.

## Files touched
- crates/ap-run/src/{builtins.rs, interp.rs, classes.rs, frame.rs}
- vendored_stdlib/functools.py (wrapper marker attr)
- probes-02resid-b/ (probe corpus, untracked)
- plans/antipythony/02-native-backend/NEXT-02RESID-B.md (this file)
