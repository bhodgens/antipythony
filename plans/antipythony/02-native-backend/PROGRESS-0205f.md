# Leaf 02.05f — Boxed OSR completion: back-edge poll + transfer buffer + OSR entry

Status: LANDED on `tier1-0205f` (base = main incl. 02.05a..02.05e).

## What landed

- **clif.rs** — three pieces:
  - `emit_cfg` gained an OSR-parameterized twin, `emit_cfg_with_osr`:
    the `Inst::OsrPoll` arm now emits the REAL back-edge poll when the
    caller passes a non-zero `fn_key` (`compile_baseline_with_osr`; plain
    `compile_baseline` keeps polls as no-ops so the tier-1 twin/fast-path
    emitters and existing unit tests are untouched). The poll arm shape:
    `entry = h_osr_poll(ctx, fn_key, header)` (i64) → NULL continues the
    loop in tier-0; non-NULL builds an `OsrTransferBuffer` in an Explicit
    stack slot (header | live_count as ONE combined i64 store at offset 0 —
    two u32 fields of the repr(C) header; separate i64 stores would stomp
    bytes 4..12, i.e. slots[0]'s Value), ZERO-fills all 16 slots (see
    Crash 1 below), fills locals (LocalId order) then live-ins (ascending
    TempId) via `h_osr_fill`, and `call_indirect`s the entry in the OSR
    ABI `(ctx, buf) -> i32` (OSR-BY-CALL: statuses 0/1 forwarded as this
    frame's own return — completed/ret parked or exception in flight;
    the tier-0 frame is abandoned). Header resolution: `osr::loop_headers`
    walked with a per-emit cursor (ascending block order = the emit
    walk's poll order); `osr_fn_key == 0` keeps the whole arm a no-op.
  - `compile_osr_entry(func, header)` — the OSR entry is a BASELINE
    compile of the SAME cfg in the OSR ABI. Prologue loads the transfer
    through `h_osr_load` using the SAME `osr::OsrPlan` table the tier-0
    poll emitter filled (the shared-constant-table rule), then jumps to
    the header's CLIF block (NOT block 0 — no twin-start dispatcher
    involvement; `ctx.twin_start` untouched). From there the ordinary
    emit_cfg machinery runs the loop body to the function's end; boxed
    return/raise terminators park ret/pending_exc exactly as a call.
    Eligibility gate at compile time: `osr_plan_resolved == None`
    (over-cap live set) → Err, reported, never silent. Polls are
    DISABLED inside the entry (fn_key = 0 — a nested transfer would
    abandon this frame's stack slot home).
  - `helper` table gains `apjit_h_osr_poll` (returns I64 — the entry
    POINTER; declaring I32 truncated the pointer and the indirect call
    jumped to garbage — Crash 2), `apjit_h_osr_fill` (Rust-side 16-byte
    Value clone from ctx.slots[src] to buf.slots[idx]), `apjit_h_osr_load`
    (entry prologue: buffer slot → ctx slot).
- **runtime.rs** — `h_osr_poll` (host-seam inverted like
  SUBSCRIPT_GET_FN: `set_host_osr_poll` installed by ap-run at JIT init;
  absent in unit tests → NULL = never OSR), `h_osr_fill`, `h_osr_load`,
  the `OSR_TRANSFERS` exercise counter + `note_osr_transfer` +
  `osr_stats_line` (`[OSR-STATS] transfers=N` under APX_OSR_STATS=1 —
  the transfer is counted at the entry prologue's first load, idx==0),
  and a cached-once `osr_dbg()` flag (APX_OSRDBG prints; no per-call
  `env::var` allocation — the 02.05e pipe-buffer lesson applies per
  back-edge crossing).
- **osr.rs** — `OsrPlan` (the ONE canonical mapping: locals ascending
  LocalId = buffer slots [0..L), live-ins ascending TempId = [L..L+V)),
  `osr_plan_resolved`, and `loop_headers` (poll-block predecessor that
  forward-reaches the poll block = the while-shape loop header; the
  v0 while lowering is single-back-edge by construction).
- **tierup.rs** — OSR-entry compiles route through the SAME background
  driver: `OsrCompileRequest` (fn_key, fn_name, header, cfg — no
  feedback snapshot needed; the entry is a baseline lowering),
  `OsrEntryHandle`/`OsrCompletion` (per-instance `OsrCompletions` map
  keyed (fn_key, header), disable marker = header blacklisted, never
  retried), `request_osr` (exactly-once via the caller-held in-flight
  set) + `poll_osr` + `call_osr_status`, and the compiler loop pins
  finished entries in `pinned_osr` (pin-until-retire discipline
  unchanged).
- **jit_hook.rs (ap-run)** — the OSR registry (`OSR_ENTRIES` +
  `OSR_REQUESTED` thread-locals, keyed (fn_key, header)): the poll seam
  `osr_registry_lookup` (installed via `set_host_osr_poll` in
  `init_tier`) returns installed entries, blacklists ineligible headers
  (the OSR_MAX_LIVE gate, reported under APX_TIER_DEBUG), or enqueues
  an OSR compile through the driver; `osr_poll_installs` drains
  finished compiles at each `jit_call_body` boundary (same poll-to-
  install discipline as tier-1). `compile_for` now compiles the tier-0
  baseline WITH live polls (`compile_baseline_with_osr(fn_key)`).
  `report_line` prints `[OSR-STATS] transfers=N` under APX_OSR_STATS=1.
- **tests**: `osr_entry_tests.rs` (5): entry compiles for a while
  header; the entry runs the loop to completion FROM a populated
  transfer (i=7/total=0/n=10 → 48, three-way consistent with the
  baseline from the top = 90); the plan ordering pinned in both
  directions (locals asc / live-ins asc / eligibility agreement);
  `h_osr_load` direct. `osr_gate.rs` (ap-cli, 2): the hot-loop probe
  OSR-transfers (transfers >= 1 under APX_OSR_STATS=1 — the exercise
  proof that byte-identity alone cannot give) and stays byte-identical;
  the >16-live probe stays identical with transfers == 0 (poll-based
  tier-0 only). `osr_tests.rs` buffer-layout pin updated for the
  16-byte-Value slots.

## Deliberate design points / divergences (recorded)

- **Slots carry `Value` directly** (16 bytes), not pon's usize word
  slots: both transfer sides are Rust compiled from THIS crate, so the
  repr(C) layout is shared by construction and no usize-punning shim is
  needed (pon packs smis into words; our boxed-baseline ABI moves
  Values).
- **OSR entries never OSR again** (fn_key=0 inside the entry): a nested
  transfer would abandon the entry's own ctx while its stack-slot home
  (the tier-0 frame) is still parked mid-call.
- **The buffer is ZERO-filled before the fills**: OSR frames stay on
  the CONSERVATIVE-scan path by design (pon discipline, kept) — the GC
  may fire mid-transfer (any helper allocation) and the conservative
  scan reads the whole native stack region including the transfer
  buffer; uninitialized stack garbage misread as Gc pointers was a
  LIVE crash (Crash 1).
- **Installs land at call boundaries only** (`osr_poll_installs` in
  `jit_call_body`): the registry lookup inside JIT'd code never blocks
  or takes the driver lock beyond the map probe; a finished compile is
  visible to the function's NEXT call, whose loop then transfers. (A
  single-call run therefore never OSRs — by design; the gate probe uses
  two calls.)
- **The tier-1 fast path is untouched**: regions exclude poll blocks
  (region.rs "osr poll" disqualifier, pre-existing), so the fast path
  never contains polls; the loop runs in the twin, whose polls work
  identically.

## Crashes found by the gate probes (fixed en route)

1. **GC crash mid-transfer** (smoke probe segfault after 2 fills): the
   transfer buffer's Explicit stack slot started UNINITIALIZED; the
   conservative root scan (legitimately walking OSR frames) read stack
   garbage as Gc pointers on the next allocation → segfault. Fix: zero
   all 16 slots before the fill stream.
2. **Pointer truncation** (segfault on the first transfer): `h_osr_poll`
   returns the entry pointer as i64, but `i64_return()` did not list it —
   helper_sig declared I32, truncating the pointer; call_indirect jumped
   to garbage. Fix: `apjit_h_osr_poll` returns I64.
3. **repr(C) header stomp**: two separate i64 stores (header at 0,
   count at 4) wrote bytes 0..8 and 4..12 — the second store's high
   half landed INSIDE slots[0]'s Value. Fix: one combined i64 store
   `(header | count << 32)` at offset 0 (little-endian = the two u32s).
4. **`let...else` non-divergence**: the fail-closed arm in the poll
   emitter used `unreachable!()` after `return_` — fires at EMIT time
   (crates panic when the plan is None). Restructured to
   `return Ok(())` (emit-time bail; the emitted code still terminates
   the frame with status 1).

## Gate evidence (SPLIT-02.05 §02.05f)

- **Hot-loop tier-up probe**: `hot(200000)` + 2 more calls — tier-0 vs
  adaptive stdout byte-identical, rc identical; `[OSR-STATS] transfers=1`
  proves the OSR entry ran (gate: `hot_loop_osr_transfers_and_matches_tier0`).
  Three probe runs rc=0 (stable). Full output matches CPython ground
  truth (39999800000 / 90 / 0).
- **Ineligible-loop probe**: 20 locals (> 16 cap) — byte-identical with
  tier-0, transfers == 0 (the eligibility gate refused the compile;
  correctness purely via poll-based tier-up at the boundary) —
  `ineligible_loop_reports_and_stays_identical`.
- **Unit suite**: ap-codegen-native 59/0 (54 pre-existing + 5 new
  osr_entry_tests); ap-run lib 4/0; tier_mode_differential 3/3 (all 17
  transparency probes mode-pair byte-identical WITH live OSR polls in
  every tier-0 compile); tierup_race_stress 2/2 (sync-mode corpus parity
  + 100-run async race stress); gc_differential 2/2 (OSR frames on the
  conservative path); ap-gc 35/0.
- **clippy `-D warnings --all-targets`** clean on ap-codegen-native,
  ap-run, ap-cli; `cargo fmt --check` clean.

## Open items (stretch / follow-up)

- **Typed-OSR** (unboxed live-ins at transfer) stays OUT per the split
  (study open problem #2): the transfer is fully boxed (pon's own
  answer); revisit only if a corpus loop profile shows re-entry churn.
- `loop_headers` resolves the v0 single-back-edge while shape only;
  For-loop OSR lands with the For lowering (02.05h bench uses For —
  OSR there is a follow-up, not a gate item).
- The registry is per-thread (thread-local): a 02.09 no-GIL world needs
  it shared or per-thread-registry lookup from JIT'd threads — filed
  for the 02.09 audit, not reachable today (single mutator).
- OSR for generator bodies (Suspend terminators) is untested; the
  lowering guards them out at `lower_fn` (generators fall back to the
  tree-walker today).
