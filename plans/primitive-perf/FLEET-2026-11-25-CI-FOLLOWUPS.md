# Fleet wave: post-CI-greening follow-ups (2026-11-25)

Delegation unit: deleg_981aab5f (4 parallel leaves, background).
Orchestrator (main session) owns: index/git state, adversarial review of
child diffs, contract arbitration, the commit queue, and CI watch/report.

## In flight

| leaf | branch | worktree | owns | task |
|---|---|---|---|---|
| ep16 | tier1-ep16 | ~/.git/worktrees/leaf-ep16 | select region of ap-run/builtins_modules.rs + select probe in stdlib_modules_differential.rs | issue #16: real select.epoll object on x86_64-linux-gnu |
| pins17 | tier1-pins17 | ~/.git/worktrees/leaf-pins17 | crates/xtask (main.rs, new pins.rs), docs/host-adaptive-pins.md, tests/PIN_HOSTS.toml, corpus_traceability lint | issue #17: `xtask pins --check/--fix` + darwin-pin survey/classification doc |
| assess15 | tier1-assess15 | ~/.git/worktrees/leaf-assess15 | ap-pack/ap-host/ap-conform/ap-codegen-wasm src (forbid attrs), scripts/unsafe_report.sh, docs/unsafe-baseline.txt, docs/unsafe-inventory.md, clippy.toml, ONE line in root Cargo.toml [workspace.lints.clippy] | issue #15: unsafe assessment + ratchet script + forbid rollout (skip ap-cli/xtask) |
| ug14 | tier1-ug14 | ~/.git/worktrees/leaf-ug14 | .githooks/**, scripts/enable-git-hooks, scripts/disable-git-hooks, docs/git-unsafe-gate.md | user directive: pre-commit gate blocking NEW unsafe rust (opt-in; do NOT enable hooksPath) |

## Cross-leaf contracts (binding)

- assess15's `scripts/unsafe_report.sh --diff` and ug14's `.githooks/pre-commit`
  are INDEPENDENT implementations of the same check on purpose (ug14 brief
  forbids calling the script). Arbitration at merge: if the script lands
  first and is hook-safe (exit codes, no interactivity), a follow-up may
  rewire the hook to call it; hooks stay self-contained until then.
- Both of assess15's + ug14's outputs reference issue #15; merge order:
  assess15 first (inventory defines the ratchet baseline), ug14 second,
  then a one-line tracker update pointing the hook doc at the script.
- File collisions on trunk: ep16 vs pins17 vs assess15 share
  crates/ap-run/src/builtins_modules.rs (ep16 only, select region) and
  crates/xtask (pins17 only) — disjoint by construction. assess15's one-
  line root Cargo.toml edit is the only shared-file touch; merge last among
  rust changes or rebase over siblings.
- fmt discipline: every leaf fmts its OWN crate only; orchestrator checks
  `git show --stat` per merge for fmt-drift on off-limits files.

## CI watch (report item #1)

- Run 45712374719 on 8ed5f91 in flight; watcher proc_b33f994df7f5 writes
  /tmp/ci_watch23_status.txt. Prior runs peeled: fmt, SIGSEGV, simd, fence
  path, packaging path, uv, dead_strip/-lm, NUL-terminated UTF8AndSize,
  stdlib linux parity (b849c9c), scratch-test sweep (be4f6d6, 8ed5f91).
- conformance pytest has NEVER run in this campaign (rust job gates it).

## Merge plan (after child reports)

1. verify-before-merge per leaf: re-run its headline gates first-hand in
   its worktree with its CARGO_TARGET_DIR (ep16: mac 48/0 + x86_64
   container differential; pins17: xtask suites + pins --check +
   corpus_traceability pytest counts unchanged; assess15: builds+tests of
   four forbid'd crates + script self-test; ug14: bash -n + staged
   scenarios transcript).
2. merge order: assess15 -> ug14 -> pins17 -> ep16 (smallest blast radius
   first; ep16 is runtime surface, gated last behind full re-verify).
3. post-merge: cargo fmt --all --check, rebuild gate artifact set
   (ap-run-eval debug, ap-run-probe release, ap-host, both CRT staticlibs,
   uv on PATH), cargo test --workspace slice spot-check, push, CI verdict.

## NOT in scope

- No push/merge by children (orchestrator owns trunk).
- No hook enablement on the shared machine (AGENTS.md hooksPath caveat).
- No mass pin regeneration (pins17 = mechanism + survey only).
- epoll OBJECT beyond CPython-core methods (EPOLLET edge-mode tuning,
  epoll_create(sizehint) semantics beyond accepting the arg).
