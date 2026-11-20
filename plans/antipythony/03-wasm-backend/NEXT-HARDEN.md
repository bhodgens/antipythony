# Leaf HARDEN — post-03 emitter hardening: the 11 LOWER-PANIC scan-vet contract violations

**Worktree:** `~/.git/worktrees/leaf-harden` (branch `tier1-harden` off main@020bb37)
**Date:** 2026-11-20
**Input:** `03.07-not-lowered.json` totals.LOWER-PANIC = 11 (leaf 03.07 section 3
blocker-class findings). Commit: f217f8a (single commit; all fixes are one
grouped increment in `crates/ap-codegen-wasm/src/lower.rs`).

## The contract

The wasm scan (`collect_*` functions) ACCEPTED source shapes the emitter
then killed the process on (`unreachable!("scan vetted ...")`, planner
`fail_closed_gen` panics, `mod_global` panics). Nothing miscompiled — the
process aborted loudly — but a scan that accepts what emit cannot lower is
a fail-closed contract violation (the project's core invariant). Fix
discipline: REJECT AT SCAN with an `Unsupported` reason naming the owning
post-03 slice (matching existing reason-string style); emit-arm
implementation was NOT used for any case (no case had a trivial arm — a
runtime getitem FN does not exist in the prelude, and C3 multi-base
linearization is a real cls_new surface change).

## Per-case disposition (all 11: scan-rejected)

| # | case | panic site (baseline) | root shape | disposition |
|---|------|----------------------|------------|-------------|
| 1 | values/V043-dict-key-errors | lower.rs:6103 `scan vetted expr` | `d[[1, 2]]` subscript read (List key) | scan-rejected: `subscript read (post-03 emitter surface)` (Expr::Subscript arm) |
| 2 | values/V063-str-repr-quotes-escapes | lower.rs:1754 `mod_global` panic | `eval(...)` read inside a genexp hidden fn — hidden-fn free refs never reach mod_slot | scan-rejected: `unresolvable free name in fn scope (post-03 emitter surface)` (post-resolve_cells pass) |
| 3 | classes/c001_class_basics | lower.rs:6103 `scan vetted expr` | `A.__mro__[-1]` Subscript(Attribute) read | scan-rejected: subscript read |
| 4 | classes/c009_nondata_shadowed | lower.rs:6103 | `c.__dict__["x"]` | scan-rejected: subscript read |
| 5 | classes/c016_slots_messages | lower.rs:6103 | `c.__dict__["x"]` | scan-rejected: subscript read |
| 6 | classes/c019_mro_examples | lower.rs:5569 `multi-base class rejected at scan` | `class C(B1, C1)` diamond | scan-rejected: `multi-base class (post-03 C3/MRO surface)` (scan_class_def; the unreachable! message already claimed scan gated this) |
| 7 | classes/c020_mro_errors | lower.rs:5569 | `class X1(DA, DA)` / `X2(IA, IB)` | scan-rejected: multi-base class |
| 8 | classes/c022_super_diamond_chase | lower.rs:5569 | `class B2(B, C)` diamond chase | scan-rejected: multi-base class |
| 9 | exceptions/e008_exceptiongroup_structure | lower.rs:6103 | `e.exceptions[0]` | scan-rejected: subscript read |
| 10 | controlflow/G001-pep342-generators | lower.rs:2744 `fail_closed_gen("gen try orelse/finally")` | `closer()`: try/finally INSIDE a generator body | scan-rejected: `generator try orelse/finally (03.04e segment slice)` (collect_stmt Try arm, keyed off parent_stack generator-ness) |
| 11 | tier1-transparency/t1_004_osr_side_effects_once | lower.rs:6103 | `marks[0]` list subscript read | scan-rejected: subscript read |

En-route hardening (same violation class, latent): AugAssign non-Name
targets — both AugAssign emit arms (plain lower.rs:3470 + gen lower.rs:4879)
unreachable! on `scan vetted augassign targets` while collect_target_read
accepted Subscript/Attribute — now rejected at scan:
`augassign target beyond a name (post-03 emitter surface)`.

## Regression test (the contract, going forward)

`crates/ap-codegen-wasm/tests/scan_vet_contract.rs` — for each of the 11
rows: parse -> scan -> lower behind `catch_unwind`; outcome must be
Ok(bytes) or Err(Unsupported) — a panic is a test FAILURE naming the case.
Any future scan/emit drift on these shapes fails there.

## Verification

- Contract test: 11/11 handled (unsupported 11, lowered 0, panics 0).
- Full inventory sweep (`not_lowered_inventory`): LOWER-PANIC 11 -> 0;
  lowers 53 -> 52, unsupported 147 -> 159 (11 former panics + e009 + 5
  reason-text refinements, see below).
- Gates held (this tree, this session):
  - CLI corpus gate: 51/51 byte-exact (fresh debug ap-host build +
    regenerated dumps via cli_corpus_dump)
  - ge_ma_probe: pass (GE001-GE005 + MA001)
  - controlflow_wasm_gate: 18/18
  - classes_exceptions_wasm_gate: pass (17/17)
  - for_probe: pass (5/5)
  - ap-host suite: 8/8

## Honest gaps / notes

1. **e009_warning_subtree is a 12th case, newly scan-rejected (behavior
   change, disclosed):** baseline it "LOWERS"ed — but its lowering produced
   a module that would TRAP at wasm runtime (`SyntaxWarning` is not in the
   emitter's EXC_TREE — only `Warning` is — so the name resolved to a
   never-written module global and the `$function` cast traps). The
   extended-corpus "50/53 byte-exact" pins compare NATIVE vs CPython only;
   e009 was never executed on wasm. Scan now rejects it via the
   unresolvable-free-name pass: fail-closed is strictly more honest than
   lower-then-trap. The 51-case wasm corpus is unaffected (CLI gate 51/51).
2. Five previously-Unsupported cases (t1_002/t1_015/V009/V013/V045) changed
   rejection reason text only (generic `Expr (03.02c slice)` ->
   subscript-read), same disposition class.
3. Not fixed (filed, out of this leaf's scope): the OTHER planner
   fail_closed_gen panics (yield nested in gen if/while/for conditions,
   handler bind-yield) have no corpus case and no scan-time gate yet — the
   contract test covers the 11 recorded rows only; extending scan-time
   gating to every planner arm is follow-up hardening.
4. No emit arm was implemented anywhere: no getitem runtime FN exists in
   the prelude (SLOT_GETITEM has no call site), so even the "trivial"
   subscript-read emit would have required new runtime surface — scan-reject
   was the only honest disposition.
5. cargo fmt reflowed lower.rs in the commit (formatting-only hunks
   interleaved with the 5 semantic edits; semantic content listed above).
