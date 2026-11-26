# HOT-PATH-PARITY — orchestrator (01-nursery)

Two parallel leaves closing the allocation floor: the ~985ns
container-mint cost and the ~1.5µs bound-method mint.

## Child index

| leaf | what | depends |
|---|---|---|
| 01-gc-nursery.md | bump arena + batch registration in ap-gc | none |
| 02-lazy-dunders.md | lazy dunders/attrs + bound-method fast path (ap-obj/methods) | none (independent of nursery) |

## Dispatch protocol

1. Both leaves in PARALLEL (file-disjoint: nursery = ap-gc +
   gcsupport; lazy-dunders = ap-obj object.rs/value.rs + methods.rs;
   dict.rs `new_in` variants are nursery-owned, lazy-dunders owns the
   empty-static singleton — no overlap).
2. Verify-before-merge each: GC differentials MANDATORY on nursery;
   all 7 diff gates + conform + cargo on both.
3. Merge order: lazy-dunders first (smaller blast radius), then
   nursery; trunk re-verify both after.

## Test Gate

GC fence invariants re-proven on nursery (gc_differential +
ast_getattr_identity_fence). Targets: container mint <= 100ns
amortized; bound-method mint <= 300ns; repeat attr read <= 100ns.

## Completion Tracking Table

| leaf | status |
|---|---|
| 01-gc-nursery.md | PENDING |
| 02-lazy-dunders.md | PENDING |

## Review Checklist
- [ ] fence invariants re-proven (nursery)
- [ ] targets met or honest gaps recorded
- [ ] no out-of-scope file edits
