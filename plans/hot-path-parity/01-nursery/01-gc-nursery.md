# 01-gc-nursery — bump arena + batch registration

## DISPATCH INSTRUCTION

Agent: implement in worktree /Users/caimlas/.git/worktrees/leaf-hp-nursery
(branch tier1-hp-nursery, branched from current main). Do NOT commit to
main; commit your own work with explicit pathspecs. Machine: Mac M2
Ultra. ONE heavy cargo lane; CARGO_BUILD_JOBS=4;
CARGO_TARGET_DIR=$PWD/target-hp-nursery on EVERY cargo command.

Parent: plans/hot-path-parity/master.md. ~90K context.

## Problem (measured)

Every container mint (PyDict::new, PyList::from_vec, output Vec->Bytes)
pays: Gc alloc 48ns + registration hook ~985ns total. The registration
hook splits as: TLS probe + drain check + 5-map critical section
(~160-250ns) + amortized collect passes (~400-700ns/mint). CPython pays
~150ns. This gates json.loads (0.31-0.46x), dict.build, and the output
mint tail of every primitive.

## Design (implement THIS, adapt as measurement dictates)

1. **Thread-local bump nursery** (ap-gc): `Nursery::alloc_room(len)` —
   Bumpalo-style cursor over a large thread-local slab (start 4MB,
   grow x2). Containers allocate `new_in(&nursery)` variants placing
   their header+payload in the room. Zero per-object registration.
2. **Batch registration**: the nursery registers ONE range entry per
   slab (a cursor object) with the GC registry; the collector treats
   the whole nursery as a single root. Per-object registration happens
   only at **promotion**.
3. **Promotion policy**: on any of — object survives N=2 minor
   collections, an external pointer is stored INTO a nursery object's
   interior from outside, slab 90% full (evacuate live objects to
   normal heap, slab resets). Promotion = allocate normal + copy +
   register + rewrite handles (the runtime routes Value handles
   through Gc<T>; a nursery object's Gc must be stable until promotion
   — use a two-level handle: handle table entry points at either
   nursery slot or heap; promotion rewrites the entry. If handle-table
   surgery is too invasive for one leaf, FALLBACK: promote on ANY
   external store attempt (conservative) and document).
4. **Fence invariants preserved**: unboxed Smi lists => zero internal
   edges + stays registered (they allocate in the nursery and promote
   like everything else); cycles within the nursery are collected by
   the normal cycle collector treating the nursery as a root set and
   tracing internally; objects with finalizers (`__del__`) promote
   immediately (destruction timing divergence is already documented).

## Files you own

crates/ap-gc/src/* (nursery, registry, collector changes), crates/
ap-obj/src/dict.rs + list.rs + object.rs (new_in variants only —
coordinate via tracker if siblings hold them; check git status),
crates/ap-run/src/gcsupport.rs. ap-cli/src/bin/ap_gc_eval.rs for the
bisect probe (it exists from Fix C).

## Tasks

### T1 — baseline + probe (before any change)
Extend the ap_gc_eval pattern: probe that mints 160k containers in
loops (dict 3-elem, list 3-elem, Bytes 1KiB) and reports ns/mint;
record the CURRENT ~985ns number on your tree.

### T2 — nursery core
Slab + cursor + `alloc_room`; PyDict/PyList/Bytes `new_in` variants;
batch registration. Unit tests: alloc/free/promote accounting.

### T3 — collector integration
Nursery-as-root tracing; promotion (evacuation) on policy triggers;
gen-0/gen-1 passes skip nursery interiors except as roots. RUN THE GC
DIFFERENTIAL SUITES (gc_differential, ast_getattr_identity_fence) —
they must stay green; these prove the fence invariants.

### T4 — wire hot minters
PyDict::new / PyList::from_vec / Bytes mint paths route through the
nursery by default (thread-local). Keep `new_unregistered`/heap paths
for long-lived singletons (interned strings, type objects).

### T5 — gates + measure
7 diff gates (fix_int, fix_json, fix_sort, fix_random, diff_b64,
diff_hex, fix_sha_zlib positional) + ap-conform values 50/50 + stdlib
48/48 + cargo test -p ap-run -p ap-gc -p ap-obj --release + GC
differentials. Re-run the T1 probe: target <= 100ns/mint amortized
(>=10x). Bench slices to SCRATCH: --only P09, --only P10.

### T6 — report
plans/hot-path-parity/01-nursery/report.md: bisect numbers, design
deviations, promotion-policy results, before/after per shape, honest
gaps.

## Self-verification checklist
- [ ] T1 baseline recorded before changes
- [ ] GC differential suites green (fence invariants)
- [ ] ns/mint <= 100 amortized (>=10x)
- [ ] json.loads dict-shape improved (re-run fix_json.py + the
      dict-mint isolation probe from fix-c)
- [ ] all 7 diff gates + conform green
