# 02-typed-dict-regions — typed dict getitem/setitem in tier-1

## DISPATCH INSTRUCTION

Worktree /Users/caimlas/.git/worktrees/leaf-hp-typeddict (branch
tier1-hp-typeddict, from current main). Own commits, explicit
pathspecs. ONE heavy cargo lane; CARGO_BUILD_JOBS=4;
CARGO_TARGET_DIR=$PWD/target-hp-typeddict everywhere.

Parent: plans/hot-path-parity/master.md. GATED: dispatch only AFTER
01-nursery + 03-jit-for-loops MERGE and a re-measure shows dict rows
still below 1.0x. LARGE leaf (4-8 sessions per the tier1 verdict doc).

## Problem

dict.getitem twin path measures ~1.0x: the primitive dominates. The
only remaining route to >1x dict rows in loops is typed codegen: the
JIT compiles `D[k]` over a dict-tagged temp into an inline
compact-dict probe (hash k, index-table walk) with a PyDict type
guard and deopt to the twin on miss/non-dict.

## Design (from the tier1 verdict doc §5.2 — read it first)

1. Region admission: allow `Subscript` over a dict-tagged temp in a
   typed region IF the region contains no Call (aliasing hazard: a
   call may mutate the dict and invalidate cached layout). This
   extends the existing has_int_hotness/eligibility rules.
2. Guard: emit a type check (PyDict tag compare) on the temp at
   region entry; deopt en-route on failure (the 02.05c cold-twin
   continuation machinery handles this).
3. Inline probe: hash the key (int keys: value itself per the
   CPython-exact recurrence; str keys: cached hash), walk the index
   table inline, compare entry key, load value. No allocation on the
   hit path.
4. StoreSubscript: same treatment for `D[k] = v` (resize on
   load-factor violation deopts to the twin — resize is rare and
   cold).

## Files you own

crates/ap-codegen-native/src/{cfg.rs, region.rs, clif.rs, infer.rs,
runtime.rs}, crates/ap-run/src/jit_hook.rs. New tests under
crates/ap-codegen-native/tests/.

## Tasks

T1 — region admission extension + guard emission + deopt wiring for
`D[k]` read. T2 — inline probe lowering (int-key first; str-key with
cached hash second). T3 — StoreSubscript typed path. T4 — measure
with the pp2_tier1 harness pattern (dict_getitem/dict_build while
loops, function-wrapped): target >= 2x CPython on the dict rows.
T5 — full gate battery (7 diff gates, conform both modes, cargo
ap-run + ap-codegen-native, GC differentials) + regression tests for
the guard/deopt paths (non-dict receiver, key-type change mid-loop).
T6 — report plans/hot-path-parity/02-dispatch/typed-dict-report.md.

## Self-verification checklist
- [ ] deopt proven: non-dict receiver and mutated-mid-loop cases
      produce identical results to tier-0 (regression tests)
- [ ] dict rows >= 2x CPython on function-wrapped while loops
- [ ] all gates green incl. adaptive conform
