# Branch 01 - PROGRESS

### 01.04 xtask tests - DONE 2026-10-20 (leaf sa-3)

`cargo test -p xtask`: was 0 tests, now 8 green (4 unit + 4 integration).

- **Unit (`src/main.rs` `#[cfg(test)] mod tests`)**: render purity +
  structure; two full in-process `collect_deps()` renders byte-identical
  (determinism claim at HEADER); entries sorted by (name, version) with a
  field-identity guard (crate names never start with a digit, versions
  always do); static_entries gated on vendored tree existence.
- **Integration (`tests/xtask_gates.rs`)**: spawns the compiled binary
  (`CARGO_BIN_EXE_xtask`) with `current_dir` + `CARGO_MANIFEST_DIR` pointed
  at a stdlib-built tempdir fixture workspace (minimal 1-member cargo
  workspace - the tool's seam is CWD/env, no args needed, no dev-deps).
  Proves `notices --check`: exit 0 in sync, exit 1 stale (drift injected in
  fixture copy only), exit 2 missing file; CLI regenerate byte-identity;
  plus a read-only assertion of the LIVE repo gate (same command ci.yml
  fast-lints runs).
- **Repo THIRD_PARTY_NOTICES never mutated**: sha256 identical before/after
  (`7db6cffc…9342b18`). No restructuring of the tool; no new deps.
- **Deliberate break**: swapped the `collect_deps` sort key to
  (version, name) -> `collect_deps_entries_sorted_by_name_version` FAILED
  (red proven), reverted, green again. rustfmt --check clean on both files.
- **Gaps**: stale-detection at CLI level is proven on a synthetic fixture,
  not by mutating the real repo graph (by design). Fixture tests invoke
  `cargo metadata` on the fixture (offline-safe: zero external deps).

### 01.03 ap-gc unit tests - IMPLEMENTED NOT COMMITTED 2026-10-20

`cargo test -p ap-gc`: was 0 tests, now 23 green (14 `#[cfg(test)]` units +
9 integration `tests/gc_invariants.rs`). ap-gc is the only crate touched;
no `[dev-dependencies]` added (stdlib-only fixtures); zero logic changes to
non-test code (diff is purely additive `#[cfg(test)]` mods + new test file).

- **Units (`gcmod.rs mod unit_tests`, 10 + `weakbridge.rs mod unit_tests`, 4)**:
  value_addr/strong_count_of arm coverage; `edges_of` registry-filtering
  (list/dict/tuple arms, gc007 tuple rule); subtract-refs seeding vs pure-
  cycle unreachability; `count_cycles` census (sc>1 peers vs sc==1 acyclic);
  contention-abort internals (SCAN_CONTENDED set/reset, no-sweep guarantee);
  gc016 alloc-since reset; threshold auto-collect + t0=0 + disable/enable
  gating; freeze/unfreeze sweep exclusion; CollectGuard registration
  suppression; GcStats census deltas. Weakbridge: guard thread-locality,
  bridge five-seam dispatch order, re-entrant-safe accessors (no double
  RefCell borrow across callee), nested-guard footgun pinned as KNOWN
  behavior.
- **Integration (`tests/gc_invariants.rs`)**: allocation accounting
  end-to-end through the gc_gate hook (dedupe, gc007, gc016, gc011 return
  units); 5-node cycle freed in one retain pass + anchored-cycle survival
  (C-22 never-false); root-dict + module-ns keepalive across collects then
  collectable after release; C-22 verify-paragraph verbatim (strong ref +
  5 forced collects => no fire; drop+collect => dead SAME pass, take-once);
  bridge ORDER probe inside collect(): `del` seam before `fire` seam, dead
  flag flipped but registry retain NOT yet run at fire-time (GARBAGE_NOW
  regression pin, gc004/5/8/15); resurrection (gc008): stays tracked,
  weakref must not fire, finalizer never re-fires (C-21 once-only);
  contention-abort via live `attrs.borrow_mut()` across collect(): returns
  0, victim surviving only through the contended object stays tracked,
  ALLOC_SINCE NOT reset (retry soon), then post-release pass sweeps the
  detached chain and a rooted chain survives every pass.
- Every test documents its INVARIANT + spec/contract refs in a comment
  (S-06-17 #1 timing ban; C-20/C-21/C-22/C-23; corpus gcNNN ids where the
  behavior is diff-pinned; D1 noted where only explicit-collect anchoring
  is deterministic).
- **Deliberate break**: `if SCAN_CONTENDED...` -> `if false` (abort disabled)
  => `contention_abort_never_sweeps_and_clears_flag` AND
  `collect_aborts_on_mid_mutation_contention_never_skips` both FAILED (red
  proven in both suites), reverted surgically from /tmp backup, 23 green
  again. rustfmt --edition 2021 --check clean on all three files.
  `cargo check -p ap-run --tests` compiles (warnings only, pre-existing).
- **Gaps**:
  - MMTk-swap-sensitive invariants not testable yet: per-OBJECT (not
    per-ADDRESS) finalizer bookkeeping under a MOVING heap (C-21 explicitly
    requires it; stage-1 FINALIZED_ADDRS is address-keyed + purged per pass
    — the swap must re-test this with object identity); no `trait Heap`
    seam exists yet so registry/collector internals are asserted directly.
  - gc.callbacks start/stop payload (run_callbacks_gen) and `__del__`
    exception (unraisable) paths are ap-run bridge behaviors — only their
    call ORDER is pinned here; content-level coverage stays in the gc
    corpus (gc006).
  - The nested-CollectGuard footgun (inner drop clears the outer window)
    is pinned as CURRENT behavior, not asserted correct — flagged for the
    02.04 stage-2 review.
  - Frames-root bridge channel is a no-op in stage 1 (weakbridge_frame_
    values returns empty by design, docs/gc-mmtk-swap.md §1); its
    keepalive path stays covered only via subtract-refs until tier-1.

