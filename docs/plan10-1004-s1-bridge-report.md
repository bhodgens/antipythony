# Leaf 10.04 report — S1 begins: the PyObject_Call-back bridge + C-visible
# instance payloads (2026-11-22)

Deliverables for plans/antipythony/10-c-extensions-stretch/orchestrator.md
leaf 10.04. Worktree: `~/.git/worktrees/leaf-1004` (branch `tier1-1004`,
off main `9658289`). Session record: NEXT-1004.md (same dir). Commits:
`c68f883` (bridge + payloads) + `be27276` (proof extensions + gates).

## What landed

1. **PyObject_Call-back bridge** (spike honest-gap #1 — the largest single
   S1 cost, previously UNMEASURED):
   - `crates/ap-ce/src/runtime_ctx.rs` (NEW): a registered-function-pointer
     seam — ap-ce cannot depend on ap-run (workspace layering is ap-run →
     ap-ce), so ap-run's `ce_bridge` installs ONE `PycallFn`
     (`runtime_pycall`, first ce dispatch, OnceLock) and the exported
     `PyObject_Call*` symbols dispatch through it. The interpreter handle
     rides the runtime's EXISTING thread-local
     (`interp::current_interp_handle`, the 02.09 per-worker accessor the
     classes.rs metaclass hooks already use) — no new thread-state concept.
   - Exported UNPREFIXED: `PyObject_Call`, `PyObject_CallNoArgs`,
     `PyObject_CallOneArg`, `PyObject_Vectorcall` (kwnames NULL; kwargs via
     `PyObject_Call`'s 3-arg form). GOTCHA found by the gate: the first cut
     exported `apx_PyObject_Call*`-prefixed symbols and the extension's
     dlopen failed silently (undefined `_PyObject_CallOneArg` against a
     binary exporting only `_apx_...`) — the source-shim model requires
     CPython-SPELLED names for anything an extension references directly;
     the `apx_` prefix is only safe for symbols OUR headers alias.
   - Faithful handles: `state::from_value`'s non-S0-kind fallback now mints
     a `CeKind::Opaque` handle carrying a strong `Gc<PyObject>` clone (the
     S0 repr-pin fallback handed C the object's repr STRING — fine for
     printing, fatal for call-backs and identity). The value-map
     (`value_map.rs`) carries the runtime-side Value for OUR
     tp_new-route instances (`CePayload::CeInstance`, take-once handoff).
   - Re-entrancy is safe by construction: C fn → registered pycall →
     `call_value_pub` → Python → C fn → … is ordinary Rust stack frames;
     no GIL to drop, no thread-state swap (PEP 703 posture).

2. **C-visible instance payloads** (spike honest-gap #2, the pydantic-core
   requirement):
   - Payload model: **side registry** (`payloads.rs`) keyed by the instance
     object's stable address (`Gc::as_ptr` = Arc data pointer), holding
     `tp_basicsize`-sized malloc'd, zero-initialized blocks. The
     alternative (opaque pointer slot ON the CeObject handle) was REJECTED:
     handles are minted per call (`from_value`), so a handle-slot payload
     either evaporates between calls or forces every bridge entry to
     thread a persistent per-instance handle. The side registry decouples
     C-payload lifetime from handle minting; `payload_alloc/payload_of/
     payload_take` are the surface (take is the future tp_dealloc hook).
   - `tp_basicsize` flows C spec → `apx_shim.c` walker →
     `apx_type_register` → `from_spec` → `SpecRecord`/`SpecSnapshot`.
   - `ce_bridge::alloc_ce_instance`: tp_new route allocates the runtime
     instance AND the payload block (key = instance Arc address), and
     `mint_slf` mints `CeInstance` handles for slot receivers so
     `tp_init`/methods/getsets resolve `apx_InstancePayload(self)`.
   - C surface: `ApxInstancePayload(o)` (header) — the pydantic-core
     `&self` shape: C code casts the returned block to its struct type.

## Proof (both byte-exact vs CPython 3.14.7, stdout + rc)

- `tests/ce-s1/callback.c` + `callback_driver.py`: R1–R9 — OneArg, NoArgs,
  tuple Call, Vectorcall, bound-method callable, Python→C exception
  round-trip (propagate → 1), CPython-exact TypeError text
  (`'int' object is not callable`). Diff: 0 bytes.
- `tests/ce-s1/payload.c` + `payload_driver.py`: P1–P7 — tp_init stores a
  C field, getset reads it, method MUTATES it C-side, two instances keep
  independent blocks, CPython-exact arity error text. Diff: 0 bytes.
- Refcount-emulation probe: `tests/ce-spike/spike.c`'s comparison was
  arithmetically impossible on ANY runtime (`r0 - r3 == 1` with r3 taken
  AFTER one DECREF; both runtimes agreed on 0, masking the bug). Fixed to
  the real INCREF×2/DECREF×1 pattern (`r3 - r0 == 1`); the probe now
  returns **1 on both runtimes**, still byte-identical.
- ap-ce unit gates (5/0): pycall registration first-wins, payload
  alloc/take/zero-init/spec-check, faithful opaque round-trip,
  CeInstance take-once semantics, error-state shape.

## Measured: PyObject_Call bridge cost (the leaf's measurement deliverable)

Looped driver `tests/ce-s1/measure_pycall.py` (200k
`callback.apply(inc, i)` calls = full C→Python→C round-trip through the
bridge; best-of-3, quiet machine, 2026-11-22 11:50 MST):

| runtime | per bridged call |
|---|---|
| CPython 3.14.7 (same source) | **0.105–0.112 µs** |
| antipythony DEBUG build | **12.15–12.24 µs** |

**≈ 110× slower per bridged call** (debug build; release typically 5–20×
faster, unmeasured — the debug binary is what this leaf builds). Against
the spike's estimate: gap #1 said "2–6 h est. for the hardest S1 members,
PyObject_Call unmeasured". **Actual mechanism cost: ~3.5 h wall** (design +
runtime_ctx + faithful handles + value map + unprefixed-export fix + both
proof extensions + gates) — the 2–6 h band HELD, arguably beat for the
mechanism itself. The 110× per-call factor is the real S1 risk to record:
pydantic-core validation loops that call Python objects per item will run
bridge-dominated; release-build measurement + a fast path (vectorskip
direct dispatch without the C-handle round-trip for runtime-native
callables) is the named follow-up.

## Gates

GREEN (executed): both ce-s1 drivers + spike driver byte-identical; ap-ce
5/0; ap-run fix_pass 2/0/1, classes 29/0, import 2/0, gc 6/0,
method_surface 34/0, metaclass_arity 1/0, metaclass_enum 6/0,
name_resolution 24/0, re 9/0, import_guard 42/0, rootscan_gc_shim 3/0,
risk4_ic_version 4/0; ap-cli stdlib_modules_differential **48/48**.
Worktree binary staging: `target → target-0304c` symlink (the known
fresh-worktree environment artifact; ap_run_eval_dyn resolves
`target/debug/ap-run-eval`).

PENDING at session end (sibling lane contention, recorded honestly): the
10.03 agent held `cargo test -p ap-cli` mid-session (17+ min run observed
via ps); `ast_getattr_fence`, `tier_mode_differential`, `lxml_differential`
and the remaining ap-cli suites did NOT re-run in this worktree — my
changes touch only the ce dispatch prefix path (ce_bridge ensure_pycall is
a OnceLock no-op until a ce import happens), so risk is low but UNVERIFIED
here.

## Honest gaps

1. **Payload blocks leak at GC**: no tp_dealloc wiring to `payload_take`;
   blocks free only at process exit. Address-recycling on the runtime heap
   could re-key a fresh object onto a stale slot's address (re-alloc
   replaces the entry; old block leaks — bounded by construction sites).
2. **110× per-call bridge factor is a DEBUG-build number**: release + a
   native-fast-path (skip the C handle round-trip when the callable is a
   runtime Value already) is unmeasured follow-up work.
3. **kwargs in PyObject_Call** converts dict→pairs; Vectorcall ignores
   kwnames (NULL only) — `PyObject_VectorcallDict`/kwnames are S1-remain.
4. **Pending suites** (above) — sibling lane, low risk, must re-run before
   10.04 merges to main.
5. **Non-Obj stragglers** (Bytes/Set/Frozen/Ellipsis Values) still repr-pin
   through from_value — none are reachable through the ce surface today;
   noted for the S1 sweep.
6. **Cost accounting includes the two gate-found bugs** (unprefixed
   exports, mint_slf payload linkage) and the refcount-probe semantics fix
   — the honest marginal rate for this shape of work was ~35 min per proof
   extension once the mechanism landed.
