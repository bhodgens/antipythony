# MMTk swap design — Gc<T> handle → MMTk reference (leaf 02.04 stage 2)

Status: DESIGN ONLY (the mmtk crate spike is deferred — see §8). Stage 1 shipped
an Rc-cycle collector (`crates/ap-gc`) that satisfies the gc/weakref corpus
slices byte-exactly; this doc is the migration plan for the tier-1 perf story.
It pairs with 02.05's root-map work (`docs/pon-study-tier1.md`, §GC roots shim).

## 1. Current state (what the swap replaces)

- `Gc<T> = Rc<T>` (`ap-obj::int::Gc` is a type alias). Every heap value sits
  behind an Rc payload pointer; addresses are stable for the object's life
  because the registry holds a strong clone.
- `crates/ap-gc` keeps a REGISTRY (`HashMap<usize, Tracked>`) of every tracked
  allocation, fed from `ap_obj::gc_gate::gc_register_pub` at container/object
  allocation sites. The registry clone is the +1 strong count that makes
  "strong_count > internal_edges" mean "externally referenced".
- `collect()` = subtract-refs + move_unreachable collapsed into one pass over
  the registry (no native-stack scanning): unreachable entries lose their
  registry clone, cycles break in the same retain pass, `__del__` fires once
  via the ap-run bridge, weakref liveness = registry membership.
- Roots today: explicit root dicts (module globals, builtins), module
  namespaces, `sys.modules` values, and the subtract-refs invariant itself
  (any external Rc clone — including native-stack temporaries — shows up as
  `sc > internal + 1`). Frames: function locals ride the native stack, covered
  by subtract-refs (weakbridge_frame_values returns empty).

## 2. The MMTk binding choice

**Bind via `mmtk-rust` binding crate (mmtk-core + a tiny `mmtk-antipythany`
plan crate), V8-style semi-space initially.**

Rationale (against the alternatives):
- *mmtk-core directly*: mmtk-core IS the binding-agnostic core; the "binding"
  is the crate that implements the `VMBinding` trait. We write that crate
  (~2-3 kloc: object model, scanning contract, collection participant).
- *mmtk-rust helper*: mmtk-core's Rust bindings are first-class (the binding
  crate IS Rust); there is no separate FFI layer needed. C bindings (mmtk-c)
  are for JVM/Julia-style hosts — not us.
- *Plan choice*: `NoCopy`/`VMLayout` semi-space (copying) is the wrong first
  move for an interpreter whose values are arbitrarily large (Rc payloads of
  arbitrary size, no fixed object size classes). **MarkSweep** (mmtk's
  `MarkSweep` plan) matches our current non-moving semantics, keeps every
  address-stability assumption in the interpreter and the weakref registry
  intact, and defers compaction to a later plan flip. Locked decision:
  MarkSweep first, generational + copying plans are a post-tier-1 flip behind
  the `VMBinding` seam (one-line plan switch at binding init).

## 3. Object model mapping

- `Gc<T>` becomes a handle struct: `#[repr(transparent)] Gc<T>(*mut
  MMTkObject)` — same 8-byte size, so the enum-of-16B `Value` layout is
  unchanged (contract: `size_of::<Value>() == 16`, asserted in ap-obj tests).
- MMTkObject header: 8-byte vtable-ish header word (MMTk requires a header per
  object for mark bits + forwarding when a copying plan flips). Our Rc payload
  already has a strong/weak count pair — during migration those two words
  become the MMTk header slot (no object-size growth).
- The `Tracked` registry maps 1:1 onto MMTk's live-object enumeration; during
  migration the registry remains as an OBSERVER (stats, get_objects,
  weakref-liveness queries) but stops holding clones — weakref liveness moves
  to MMTk's `has_object died` notification (§5).

## 4. Root scanning (the pon shim pattern)

`docs/pon-study-tier1.md` §roots: pon solved native-frame roots with a
safepoint map + explicit registration; *root enumeration for native frames is
entirely ours*. Our version:

1. **MMTk Scanning mutator thread context**: implement `Scanning::
   scan_roots_in_mutator_thread` to walk:
   - `ROOT_DICTS` / `ROOT_MODULE_NS` (already enumerated by ap-gc — the
     existing `add_root_dict`/`add_root_module_ns` feeds stay),
   - `sys.modules` values via the existing weakbridge hook,
   - **native-stack temporaries**: this is the part subtract-refs covered for
     free. With MMTk there is no strong-count to observe, so the interpreter
     needs a conservative **shadow stack**: a thread-local `Vec<Gc<dyn
     Rooted>>` pushed at every `Value` bind in `eval_expr` hot paths — OR the
     pon-style safepoint map (Rust stack scanning with `stackmap.rs`-
     equivalent: return-address → live-set table). Decision deferred to
     02.05: shadow stack first (simple, exact, ~10% interp overhead), pon
     stackmap second (zero steady-state cost, needs the lowering to emit
     live-sets — the 02.05 root-map work).
2. **Global roots**: `scan_vm_specific_roots` covers the static tables
   (interned types, fn registry, module cache) — all already enumerated as
   `weakbridge` hooks or `ROOT_*` lists.

## 5. Weakref semantics under MMTk

- CPython/our weakref model: a ref is (target-address, callback, dead-flag).
  Under MMTk the address is the object reference; liveness is decided by the
  collector.
- Use mmtk's **reference processing** (`ReferenceProcessor`): register each
  weakref as a `PhantomReference`-equivalent; MMTk reports discovered
  references after mark, exactly where our `sweep_weakrefs` +
  `fire_dead_weakrefs` bridge sits today. The ap-run callback invocation
  (interpreter re-entry for `__del__`-adjacent weakref callbacks) stays in
  ap-run — the MMTk side only flips dead flags and queues (addr, callback)
  pairs onto the same bridge channel.
- The `<target_addr>` token model (WeakSet/WKD dedupe by target) carries over
  unchanged: MMTk object references are stable per object lifetime within
  MarkSweep (non-moving), and after a copying-plan flip the reference VALUE
  changes but the dedupe key becomes the MMTk object id — the token attrs
  (`<target_addr>`) must switch to that id at the same flip.

## 6. Heap sizing + triggers

- Initial heap: `min(4 GiB, 25% of RAM)`, growable to 75% — mmtk options
  `heap::initial_bytes` / `heap::max_bytes` set from the binding init; env
  override `APX_MMTK_MIN/MAX` (the existing `APX_GC_THRESHOLD` env maps to
  mmtk's `ignorable::trigger` threshold ratio during migration; the Python-
  visible `gc.set_threshold(n)` translates to "soft-notify after n tracked
  allocations" via a counter check in `gc_register_pub` — semantics preserved
  for the gc016 corpus pin, real collection frequency owned by mmtk).
- Stress/gc-determinism: `APX_MMTK_STRESS=1` → `stress_factor=1` (collect every
  allocation) for the differential corpus.

## 7. Migration steps (ordered, each independently green)

1. **Handle swap, non-collecting**: `Gc<T>` becomes the MMTk handle; alloc
   routes through `mmtk::memory_manager::alloc`; no collection enabled
   (unbounded heap). Everything else unchanged — registry still observes via
   `gc_register`. Gate: full workspace + corpus identical.
2. **Root shim**: implement `Scanning` for the binding using ONLY the explicit
   root sets (dicts/module-ns/sys.modules) + shadow stack for frame locals.
   Gate: a heap-limited run survives `tests/corpus/gc/` + the tier1
   transparency probes (deep recursion allocates → forced collections).
3. **MarkSweep activation**: enable collection; move weakref liveness onto
   mmtk reference discovery; keep the registry as an observer for
   `get_objects`/`get_stats`. Gate: gc corpus slice green; the existing
   differential suites byte-match.
4. **`__del__` + finalization ordering**: mmtk `ReferenceProcessor` +
   our once-only `FINALIZED_ADDRS` guard; resurrection semantics (C-21) —
   a resurrected object must be re-marked reachable in the SAME collection:
   mmtk supports this via `object_reference_treatment` /
   post-mark re-scan; verify with gc008.
5. **Shadow stack → pon stackmap** (optional, 02.05): replace the shadow
   stack with the lowering-emitted live-set table. Perf gate: bench_report
   regression < the shadow-stack baseline.
6. **Plan flip** (post-tier-1): MarkSweep → GenImmix/semi-space; weakref
   dedupe keys move to mmtk object ids; compacting-GC pin re-check on the
   identity corpus (V-series uses address-based hashes — a moving collector
   changes `hash(obj)` mid-run, which is CPython-legal but corpus-visible;
   audit before flipping).

## 8. Spike-deferred note

The mmtk crate spike (steps 0-1: pull `mmtk-core`, stub the binding, alloc
route) needs network access to fetch the mmtk-core dependency tree; this leaf
ran network-isolated, so the spike is DEFERRED to the 02.05 window (which
needs the same tree for the root-map work). Everything above is written
against mmtk-core 0.3x API shapes as documented (VMBinding/Scanning/
ReferenceProcessor names); the spike must re-verify trait names before step 1.

## 9. What does NOT change

- The ap-run bridge seams (`set_bridge`, weakref callbacks, `__del__`
  unraisables, atexit drain) — the collector-agnostic interpreter boundary.
- The Python-visible `gc` module surface (gcsupport.rs) — it queries state
  through ap-gc's public fns; only their implementations change.
- The corpus. gc001-gc017 + the gc/weakref/unittest differential battery are
  the swap's exit gate, byte-exact, timing-masked.
