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


### 01.01 XPASS ceiling gate + pytest reference discovery - DONE 2026-10-20 (leaf 01.01)

Two coupled deliverables (branch-01 orchestrator + master.md contracts
"XPASS ceiling" and "Reference python discovery / Skip policy").

**(A) XPASS ceiling.** `tests/XPASS_CEILING` = **74** (line-1 integer +
`# measured` provenance comments). Measurement trail:

- Last VALID complete `pytest tests/ -q` tally on a working tree:
  `/tmp/ap_b3_pytest.log` (2026-10-19 14:08, pre-Wave-B commits but the
  impl binaries ran fine): `31 failed, 1261 passed, 95 skipped,
  693 xfailed, 74 xpassed`. The 74 xpasses are exactly the
  STALE-XFAIL-LEDGER "STILL STALE (74 tests)" t08 envelope (owner 02.05).
  The review's quoted 126 predates the 52-test Wave-A flips - not re-used.
- 2026-10-20 live re-measurement (HEAD 15e7480 -> e960be0 during the run,
  + sibling 02.07 threading uncommitted): **0 xpass** across every
  measurable test:
  four disjoint partition probes = 2157 tests (t01-t03 22p/179xf;
  corpus_traceability+t05+t07 781p/1sk/27xf; t04+t06+t11+fstrings
  629p/5sk/192xf; t10+t14+t15+t16 123p/58xf; t09 89p/50xf) plus three
  full-suite runs (mine /tmp/ap_pytest_0101.txt to ~63%, sibling waveA
  ~63%, sibling 00.03 ~60%) with ZERO 'X' chars through ~60-63% marks.
  The 74 t08 markers read as timeout-xfails TODAY because sibling 02.07
  threading work hangs `antipythany run` (001_hello times out at 60s;
  t08 cross-mode legs burn their 120s APX_T08_TIMEOUT each). That is a
  broken-stack measurement, not a behavior improvement - tests/README
  forbids recording baselines from it. 74 stays the contract number;
  once 02.07/02.05 land and t08 runs again, the 74 sit AT the ceiling
  (green) and 01.02 flips lower it per wave. Orchestrator may re-measure
  at a green full run and lower the file in one commit per flip wave.
- `tests/tools/_xpass_gate.py` (new): post-suite gate; parses the -q
  summary line ("N xpassed") + -rX per-test XPASS lines (max of both),
  compares to the ceiling. Exit 0 at/below, 1 above (CEILING BREACH msg),
  2 on bad input (missing/non-integer ceiling; truncated report with no
  summary line - refuses to infer 0 from a half report). Tested against
  the real b3 report (0), a doctored +1 report (1), ceiling-lowered
  variants, stdin mode, -rX extras, truncated (2).
- `tests/corpus_traceability/test_xpass_ceiling.py` (new): does NOT
  re-run the suite (~1h). Test 1 lints the ceiling FILE shape (bare int +
  '# measured' provenance) - runs in fast-lints' existing
  `pytest tests/corpus_traceability -q` step (476 passed, 1 skipped).
  Test 2 gates a CACHED report when `XPASS_REPORT=<file>` is set, else
  skips with the wiring pointer. Verified: skip / pass / breach modes.
- CI wiring for the orchestrator (ci.yml NOT edited here, per rules):
  in the `conformance` job change the pytest step to
  `python -m pytest tests/ -q --no-header | tee /tmp/pytest-report.txt`
  (needs `set -o pipefail` if kept red-faithful) and append a step
  `- name: XPASS ceiling gate (01.01)` / `if: always()` /
  `run: python tests/tools/_xpass_gate.py /tmp/pytest-report.txt`.
  Also: the conformance job's "Compat shim for legacy hardcoded REF_PY"
  step (~line 172) carries its own comment "DELETE this step when
  t05/t10 honor AP_CONFORM_REFERENCE_PYTHON" - they do now; orchestrator
  should delete it (on Linux CI the shim would satisfy the non-strict
  host-fallback rung; with AP_REQUIRE_REFERENCE=1 CI is strict anyway).

**(B) pytest-side reference discovery (closes 00.02's honest gap #1).**
`tests/tools/_ref_python.py` (new) - Python twin of the Rust helper
(crates/ap-run/tests/common/mod.rs): ladder `AP_CONFORM_REFERENCE_PYTHON`
(version-string-checked, warn+fall-through on non-3.14) -> PATH
`python3.14` (checked) -> `/opt/homebrew/bin/python3.14` host fallback
ONLY when AP_REQUIRE_REFERENCE is off (CI strict = env->PATH only, no
hardcoded paths, same as the Rust helper). AP_REQUIRE_REFERENCE=1 +
nothing resolves -> pytest.fail(canonical
"reference interpreter not found (AP_REQUIRE_REFERENCE=1)"); else
pytest.skip. Cached per process; `pytest` imported lazily so generators
could reuse `discover_reference()`.

Converted suites (module REF_PY/PYREF constants -> in-test
reference_or_skip()): t05_values, t06_classes, t07_gc, t08_tier1,
t09_binary, t10_threading, t11_stdlib, t16_hybrid, fstrings_corpus (its
old local helper's bare-`python3` fallback - a rule-5 violation - is
gone), corpus_traceability/test_golden_provenance (skipif module gate
replaced by the shared policy; non-strict behavior unchanged: skip),
t04_ir_verify (constant was doc-only -> lazy discover_reference()).

**Gates:**
1. post-conversion green: `pytest tests/corpus_traceability tests/t05_values
   tests/t07_gc -q` -> 781 passed, 1 skipped, 27 xfailed, 0 failed; plus
   t04+t06+t11+fstrings 629p/5sk/192xf/0X and t09 89p/50xf green;
   --collect-only clean across all 10 converted files (1579 tests).
2. `env -i PATH=/usr/bin:/bin AP_REQUIRE_REFERENCE=1 pytest tests/t05_values`
   -> 47 loud failures w/ the canonical message; t07 same (17); bad-env
   override + strict + scrubbed PATH -> canonical fail (fall-through then
   nothing resolves).
3. same scrub WITHOUT the flag -> 47 passed (t05) / 17 passed (t07):
   host-fallback keeps local behavior identical to before (previously
   hardcoded to the same binary). golden_provenance non-strict still SKIPS
   (verified by the 1 skip in gate 1).
4. ceiling script real/doctored/+1/lowered/stdin - all exits correct (above).
5. py_compile clean on all 14 touched/created .py files.

**Honest gaps:**
1. t09/t16 xfail(strict=False)-marked tests that hit reference_or_skip()
   report a missing-reference FAIL as "xfail" in a strict CI run (the
   marker swallows it). The strict-FAIL proof holds for the unmarked
   differential tests. 01.02's flip-to-strict protocol removes the
   swallow; until then the ceiling gate covers silent xpass drift.
2. Prose/comment mentions of /opt/homebrew/bin/python3.14 remain in suite
   HEADERS (t05:14, t10:14, t11:22, t04:11-14, fstrings:7, t01:40) and in
   provenance re-record hints - not discovery logic. Deliberate: editing
   `tests/t07_gc/_gen_expected.py`/`tests/tools/_gen_meta.py`/
   `tests/t04_ir_verify/gen_expectations.py` changes their sha256, which
   the 03.01 generator-sha lint pins in 228 .meta files - regenerating is
   03-branch work (attempted, measured the breakage, reverted clean).
3. No live CI proof (ci.yml orchestrator-owned); exact wiring + shim
   deletion documented above.
4. Ceiling is one integer for the matrix (ubuntu+macOS). If the 02.05-era
   t08 flips land different counts per OS, the orchestrator may need
   per-OS ceilings - not implemented (contract says one file, one int).
5. t15 notices-gate failure seen in the t10-t16 probe (rc=1) is sibling
   02.07's uncommitted Cargo.lock/manifest edits vs THIRD_PARTY_NOTICES -
   not this leaf, not reference-discovery related.
