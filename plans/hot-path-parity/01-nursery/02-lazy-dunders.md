# 01-lazy-dunders — PyObject dunders on demand

## DISPATCH INSTRUCTION

Worktree /Users/caimlas/.git/worktrees/leaf-hp-lazydunders (branch
tier1-hp-lazydunders, from current main). Own commits, explicit
pathspecs. ONE heavy cargo lane; CARGO_BUILD_JOBS=4;
CARGO_TARGET_DIR=$PWD/target-hp-lazydunders everywhere.

Parent: plans/hot-path-parity/master.md. ~60K context.

## Problem (measured, fix-c-report.md Root 3)

PyObject::new allocated a REGISTERED dunders dict, deep-cloned it
(2nd alloc), dropped the original — ~600ns waste per mint. Post-Fix-C
the double-alloc is gone (new_unregistered), but EVERY instance still
carries an attrs/dunders dict it usually never reads, and bound-method
mints (every attribute read on an instance) pay dict-walk costs.
BoundMethod mint total ~1.5µs -> dict.get row -99%.

## Design

1. **Lazy dunders**: PyObject holds `dunders: OnceCell<PyDict>` (or an
   enum Empty/Full). Empty is a zero-cost static; allocated only when
   a class actually defines a dunder OR the instance is first asked to
   store one. All readers (`special_lookup`, getattr chains) handle
   Empty as miss.
2. **Lazy attrs**: same treatment for instance `__dict__` where the
   runtime has one — instances without stored attributes share a
   static empty until first store.
3. **BoundMethod mint path**: audit the bound-method construction
   (methods.rs) — cache the bound method per (instance, method-name)
   in a small per-instance table ONLY when the same pair is fetched
   more than once (or use the interned-name handles from value.rs:11
   as table keys). Target: repeat attribute read on the same instance
   = one hash probe, no allocation.
4. Coordinate with 01-gc-nursery (parallel leaf): if dunders/attrs
   dicts become rare, the nursery's job gets easier; no API coupling
   required — keep changes independent.

## Files you own

crates/ap-obj/src/object.rs, value.rs (interned handles reuse),
dict.rs (empty-static singleton), crates/ap-run/src/methods.rs
(bound-method mint + special_lookup readers). Check git status for
sibling holds.

## Tasks

T1 — baseline probe: ns per bound-method mint + per attribute read on
a plain instance (fixc_dict.py pattern); record before.
T2 — lazy dunders + lazy attrs with OnceCell/enum; all readers
updated; unit tests for define-after-read ordering.
T3 — bound-method fast path (interned-key table).
T4 — gates: 7 diff gates + conform 50/50 + 48/48 + cargo -p ap-run
-p ap-obj. Targets: bound-method mint <= 300ns (from ~1.5µs),
attribute read repeat <= 100ns; dict.get bench slice to SCRATCH.
T5 — report plans/hot-path-parity/01-nursery/lazy-dunders-report.md.

## Self-verification checklist
- [ ] baseline mint/read numbers recorded before changes
- [ ] special_lookup + getattr chains handle Empty (no behavior
      change: define-after-read, __getattr__ fallback, MRO order)
- [ ] bound-method mint <= 300ns; repeat read <= 100ns
- [ ] all gates green
