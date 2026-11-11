# 02.09b — GC topology + THE FLIP (the leaf record)

Base: fd53fa54 (02.09a complete: confinement, Arc refcounts, GilSendPub retired,
mro freeze, cached_hash atomic).

## Design (locked against SPLIT-02.09.md decisions 1–3)

### P6 — GC topology (provable under the GIL, lands first)

The collector's state lived in `thread_local!` cells in gcmod.rs (REGISTRY,
SEQ, SEQ_OF, YOUNG, MID, ROOT_DICTS, ROOT_MODULE_NS, ROOT_SEEDS, THRESHOLDS,
ALLOC_SINCE, SCAN_CONTENDED, CALLBACKS, FROZEN, WEAKREFS, FINALIZED_ADDRS,
GARBAGE_NOW, STATS, WEAK_SEQ). Every accessor already went through
`.with(|c| ...)` seams, so the conversion is mechanical at the accessor
level: each `X.with(|c| c.borrow())` becomes a short-lived `X.lock()`
through a `static X: Mutex<...>` (process-global OnceLock behind a fn, per
the no-thread_local-for-cross-thread-visible-state rule).

Key invariants preserved byte-exactly:

- **Subtract-refs seeding** reads `Arc::strong_count` — atomic since 02.09a
  decision 1, sound under concurrent clone/drop.
- **SCAN_CONTENDED / contention abort** (gcmod.rs:43-45 semantics,
  `contention_abort_never_sweeps_and_clears_flag`): the scan sets the flag
  when an interior borrow probe fails; collect_gen_inner checks it after the
  mark phase and aborts WITHOUT sweeping, clearing the flag. Under the flip
  this becomes the primary soundness mechanism for the first flip (decision
  3: "run try-collect with abort-on-contention"): a mutator mid-mutation of
  a PyObject/PyDict makes the collector's try-scan fail -> abort -> retry at
  the next threshold. False-RETAINED is benign; the flag's direction makes
  false-COLLECTED unreachable for borrow-visible surfaces. The test pins the
  same bytes; only the flag's storage becomes process-global (a collection
  running on thread A must observe contention caused by thread B).
- **Registration ordering**: `gc_register` inserts (addr, seq) then YOUNG.
  Per-object mutexes never release INTO a collect mid-insert: registration
  holds the registry mutex for both inserts (same critical section as the
  old single borrow_mut), so the registry never holds an entry without its
  SEQ_OF row and vice versa.
- **Finalizer order** (gc002): the garbage vec still sorts by seq under the
  registry lock; finalizers run AFTER the lock releases (user code never
  runs under a GC lock — borrow-flag panics would deadlock the census).
- **The scratch slot** (C6) stays per-thread (per-collecting-thread reuse;
  it is pure scratch, never cross-thread visible).
- **weakbridge BRIDGE**: fn pointers are process-global constants once set
  (set_bridge at run entry); moves to a OnceLock. IN_COLLECT guard becomes
  process-global DEPTH: while any collect is mid-pass, registration is
  suppressed process-wide (this was per-thread only because the GIL made
  collect + allocation mutually exclusive; suppression is the conservative
  superset and registration is retried at the next alloc anyway).

### The mutator census (the leaf's core edit)

New: `ap-gc/src/census.rs` — a process-global table of live mutator threads.

```
static MUTATORS: Mutex<Vec<MutatorEntry>>
struct MutatorEntry {
    thread_id: u64,          // ap-run current_os_tid at registration
    active: Arc<AtomicBool>, // false when the thread exits -> pruned
}
```

- `census::register()` / `census::unregister()` — called from
  gcsupport::install_worker_thread / the worker exit path (main thread
  registered by run_module). Registration is idempotent per OS thread.
- `census::iter_active()` — the collector's walk list (returns a snapshot
  Vec<Arc<AtomicBool>>; entries whose flag reads false are skipped).
- **Per-thread rootscan capture points**: rootscan.rs's CAPTURE_POINT stays
  thread_local (it IS per-thread state — the JIT frame of THIS thread), but
  the collector no longer reads only its own: each registered mutator's
  capture point is read from the collector via a per-entry
  `capture_slot: Arc<AtomicUsize>` that jit_hook's set/clear seam mirrors
  into (the thread writes its own slot; the collector reads every slot).
  `mapped_frames_from` gains a from-foreign-FP entry point (the existing
  walker already takes the fp as an argument; only the CAPTURE_POINT read
  is per-thread). 02.05g's map/fp-chain machinery is NOT redesigned.
- **Registry drain at collect**: `drain_worker_registrations` runs at the
  top of every collect (not only at main-thread turns), so a worker's
  forwarded allocations are visible before the pass scopes YOUNG.

### Decision 2 — the (c)-surface lock story (measured against real contention)

T121's shared writes go through `with lk: out[...] = v` — the workload
itself is embarrassingly parallel; contention is on interned strings/builtins
Arc drops and the executor's per-interp tables. Concretely:

- `PyDict.state` / `PySet.state`: RefCell -> `Mutex` (parking). Epoch
  counter stays atomic; `mutation_epoch()` is lock-free (it was the GC's
  cache key; the seed cache replays it without the dict lock).
- `PyList.items` + `allocated` move under the SAME Mutex (audit entry 26:
  the invariant "allocated matches items' capacity" must not tear);
  `items()` hands back a real MutexGuard mapped to `&[Value]`.
- `PyObject` RefCell fields (attrs/func/self_value/cause/context/dunders/
  slice_parts/type_ref/gen_id...): RefCell -> Mutex where the GC probes
  them (attrs/func/self_value/cause/context/dunders) — the GC's try_borrow
  probes become `try_lock()` (same direction: contended => SCAN_CONTENDED).
  Read-heavy single-word fields that only the owning mutator writes stay
  RefCell IF provably per-thread (gen_id: written before first publish).
- `Interpreter` executor interiors: frame_stack/registry/iterators/
  generators/super_stack/tb_frames/stdout/current_exception/
  pending_super_ctx -> Mutex (the ones workers touch per call).
  `depth` (recursion counter) is per-thread by construction post-flip: each
  worker's enter_call interleaves only with its own frames — it stays a
  Cell and becomes per-thread state at the census (each mutator entry
  carries its own depth cell).
- `FnRegistry.entries/by_path/scope_tables/scope_cache`: RefCell -> Mutex
  (importer extends while workers read fn_entry per call).
- The existing "try_borrow + 50µs sleep retry" loops (02.07-era contention
  workarounds) dissolve into plain `lock()` — under the flip there is no
  GIL holder to starve; the Mutex parks.

### P7 — the flip

- `GIL`/`GilState`/`acquire_gil`/`release_gil`/`GilGuard` DELETED, plus the
  53 call sites. threading_call.rs release/re-acquire pairs dissolve (the
  primitive Mutex+Condvar waits stay). interp.rs daemon-wait keeps
  `wait_for_non_daemons` minus the bracket. worker_bootstrap drops
  acquire/release, keeps install_current_interp + install_worker_thread.
- The `APX_GIL=1` compat flag: NOT introduced. The orchestrator's note says
  it exists "until T121 is green, then deleted"; deleting the GIL outright
  in the same leaf satisfies both states — the flag has no surface to keep.
- GC trigger points (gc_gate hooks on every container alloc) run on every
  mutator now; `collect_gen` serializes PASS execution with a process-global
  COLLECT_MUTEX: the collecting thread scans while other mutators keep
  allocating (registering under the registry lock is safe — inserts are
  invisible to the pass because scope snapshots are taken under the same
  lock), and concurrent mutator-side auto-triggers BLOCK on the pass
  instead of racing two collectors (quiesce-at-rendezvous per decision 3;
  a blocked mutator at the rendezvous is a safepoint by construction).

## Gate legs (all four)

1. `test_scaling_gate_3x_on_4cores_antipythony` >= 3x GREEN -> flip the
   @xfail_ap mark off (gate mark flip ONLY — no weakening).
2. Threading slice T101–T115 + T120/T122 byte-exact (order-independent).
3. Full conformance corpus green (confinement is observation-invisible).
4. gc slice green — gcmod.rs:43-45 SCAN_CONTENDED semantics byte-identical
   (`contention_abort_never_sweeps_and_clears_flag` must not change bytes).

Plus `cargo test --workspace` 0 failed, race-stress 100/100,
tier_mode_differential 3/3.
