# Wave-3 memperf consolidation — deduped, severity-ranked

**Date:** 2026-11-07. **Repo state checked:** main at `f2fef14`; in-flight
branch `wave3-merged` at `b9ee5f7`; in-flight g1 stack `2335be7..95bbfaf`
(not merged anywhere, never gate-green — see caveats).

**Scope:** the six read-only memory/performance auditor reports from the
2026-11-06 memperf bughunt fan-out (delegation `deleg_6f737957`):

| Auditor | Scope |
|---|---|
| 0 | interp.rs / frame.rs / iter.rs / generators.rs hot loop |
| 1 | ap-obj + ap-gc object model and collector |
| 2 | importer.rs, ap-parse, ap-ir |
| 3 | builtins.rs, builtins_modules.rs, methods.rs, classes.rs, format.rs |
| 4 | codegen / packaging / host (ap-cli, ap-pack, ap-host, ap-codegen-*) |
| 5 | threading / fiber / lxml |

**Dedup baseline.** The auditors ran at `9662de2`. That baseline is OLDER than
the landed wave-3 fixer stack, so many findings were already fixed while the
auditors read. This table closes against:
- OOM campaign #2 (`docs/oom-fix-verification-2026-11-06.md`): F1-F4, G1-G10,
  M5 — closed.
- Landed on main: `2b8f41c` (A3 debug-flag bitset), `a1f8f52` (F1 purity
  order), `73fe13b` (C1 weakref O(1)), `bc01e84` (D1 bytearray), `e831fa4`
  (D2 list), `bc73b87` (D3 str subscript), `c241244` (D4 singledispatch),
  `ae53839` (D5 dict), `529ad9e` (D9-subset), `963c9c7` (A5-partial bare
  StopIteration), `40d45e7` (F1-sort), `0858b53` (iter deque/registry/fence),
  `0eff19f` (F2/F3/B4 engine-reuse, rename-merge, compile-cache seam),
  `f2fef14` (g5 D6-D9 surface work).
- In-flight `wave3-merged` (not on main yet): **g2** = `8e86c11` (B2, B3, B7,
  F5 parser/IR quadratics); **g3** = `2148bfe` + `0271294` (C2-C10, A4 GC and
  object layer); **g4** = `1b6955b` `78c89a5` `0be26d2` `4101ea5` `b9ee5f7`
  (E1-E9 threading eviction, XPath self-carrier, fiber stack pool, JOIN_CV).
- In-flight **g1** stack (interpreter hot path; `2335be7` "A1/A2/A5-tb/A6/B1/
  B5/E8/M2-adj", `d016c21`+`cf39f47` module-cell retire, `95bbfaf` merge-exact
  scope fold). Forensically verified clean-conflict onto wave3-merged but
  NEVER gate-green (provider outage kill). Marked IN-FLIGHT-g1 below; treat
  with the same trust level as an unverified patch.
- Disclosed residuals (open, characterized only): M1-M4 threading tables,
  M10 FILE_SRC, M11 XPATH_OBJECTS, M12 PENDING_GEN_ERROR, closure-cell
  ~3MB/1k.

**Status legend:** OPEN = no landed or in-flight fix covers it. IN-FLIGHT-gN =
fixed on a branch not yet on main. CLOSED-by-<sha> = verified on live main.
DUP = merged into another row. A `needs re-verify` tag marks a row whose
auditor-era citation drifted or whose live code no longer shows the quoted
pattern (presumed closed, not proven).

---

## 1. Findings table (HIGH first)

### HIGH

| ID | File:line (live) | Defect | Status | Fix shape |
|---|---|---|---|---|
| C01 | interp.rs:2066 (`note_py_loc`), called at eval_expr entry (interp.rs:2876+) | String clone of script_path + line-table scan on every expression node; biggest per-op tax | IN-FLIGHT-g1 (A1) | Cell-stored (ptr,len) pair; record only when a diagnostic path can consume it |
| C02 | interp.rs:7525 (`entry.def.clone()`); frame.rs `fn_entry` `.cloned()` | Full function AST deep-cloned twice per call | IN-FLIGHT-g1 (A2) | Registry stores `Rc<FnEntry>` / `Rc<FnDef>`; read by reference |
| C03 | interp.rs:414-431 (`extend_current`), `merge_scopes_static` ~446 | Per-import rebuild of whole FnRegistry (O(M²) AST clones) + leaked merged scope tables (GC-immune) | IN-FLIGHT-g1 (B1/A0#3; DUP-aud2-H2/H3) | Long-lived `RefCell<Vec/Rc>` registry, append in place; in-place scope map, drop Box::leak |
| C04 | ap-cli main.rs:496-526; importer compile path | No compile work is ever read back from the cache entry (aud2-H1 = aud4#4/B4) | CLOSED-by-0eff19f (seam landed); full IR read-back deferred to leaf 02.06 | Serialized `ap_ir::Module` on hit; post-02.06 exec real artifact |
| C05 | ap-obj object.rs:124-187 | Every `PyObject::new`: ~4 heap allocs + 3-4 GC registrations (~12 hash inserts) — per-allocation hot path | **OPEN** | Lazy `Option<Gc<PyDict>>` attrs/dunders; single-probe registration; slab/arena (fits MMTk design) |
| C06 | gcsupport.rs:252 | Weakref deref full-heap clone-scan per `ref()` | CLOSED-by-73fe13b | (done: O(1) registry get — verified live) |
| C07 | ap-gc gcmod.rs:291-332, 571 | `edges_of` deep-clones each traced dict/set during every GC pass | IN-FLIGHT-g3 (C2, `0271294`) | Borrow-guarded `items_ref()` iteration; no snapshots |
| C08 | gcmod.rs:513-535, 677, 751; importer.rs:250 | Root seeds re-snapshotted for whole heap every collect (incl. gen-0); FROZEN clone per pass | IN-FLIGHT-g3 (C3) | Persistent seed list invalidated on root change; borrow not clone |
| C09 | gcmod.rs:942-947 | `sweep_weakrefs` O(WEAKREFS × GARBAGE_NOW) linear contains + Vec clone per pass | IN-FLIGHT-g3 (C5) — verified still present on main | HashSet of died addrs once per pass |
| C10 | ap-parse lib.rs:106-112 (`offset_to_line`) | `collect_stmt_lines` quadratic newline rescan per statement, every import (aud2-H4/B2) | IN-FLIGHT-g2 (`8e86c11`) — verified still present on main | Build line-start index once, binary search (reuse `Lowerer::line_at`) |
| C11 | interp.rs:355 `scope_of`; merge keying (`interp.rs:446+`) | Scope tables keyed by bare name → cross-module collisions; O(scopes) `path.join` per miss, no negative cache (aud0#13 = aud2-H5/M4 = B5) | IN-FLIGHT-g1 (95bbfaf merge-exact fold + negative caching) | (in g1) per-module keying; memoized misses; index by joined path |
| C12 | builtins_modules.rs ~:5989 | bytearray mutating ops round-trip whole buffer 3× (aud3-H1/D1) | CLOSED-by-bc01e84 | (done: in-place buffer writes) |
| C13 | builtins_modules.rs pylist arms | list insert/pop/remove/reverse rebuild the whole list per op (aud3-H2/D2) | CLOSED-by-e831fa4 | (done: in-place `Vec` ops via value.rs helpers) |
| C14 | methods.rs str subscript | `s[i]` materialized `Vec<char>` per access (aud3-H3/D3) | CLOSED-by-bc73b87 | (done: ASCII fast path) |
| C15 | builtins_modules.rs singledispatch | Whole impl-registry HashMap cloned per dispatch (aud3-H4/D4) | CLOSED-by-c241244 | (done: dispatch inside one borrow) |
| C16 | builtins_modules.rs dict arms | dict methods materialized whole dict per call (aud3-H8/D5) | CLOSED-by-ae53839 (update/copy); view-construction snapshot remains → L05 | (residual folded to LOW) |
| C17 | builtins_modules.rs strip/translate | O(n·m) char-set scan per char (aud3-H5/D6) | CLOSED-by-f2fef14 | (done) |
| C18 | builtins_modules.rs `wrap_text` | Re-counts whole current line per word (aud3-H6/D7) | CLOSED-by-f2fef14 — verified live (cur_len counter at ~:5670) | (done) |
| C19 | builtins_modules.rs Incremental.decode | Full buffer clone per chunk, O(n·k) (aud3-H7/D8) | CLOSED-by-f2fef14 — verified live (prefix-drain, D8 comment) | (done) |
| C20 | ap-ir verify.rs:1748-1790, 176-178, 665-667 | Verifier deep-clones child subtrees (O(n²) peak) + full enclosing-leg String clone per item (aud4#1/#2 = B3) | IN-FLIGHT-g2 (`8e86c11`) — verified still present on main. **Conflict flag:** auditor 2 says no importer/interp call site reaches `ap_ir::verify` (test-only); auditor 4 says it runs per module. needs re-verify which path is live. | (in g2) borrowed-children stack walk; `Rc<EnclosingLeg>` sharing |
| C21 | ap-pack purity.rs:45-68 | Nondeterministic refusal names first `readdir` hit (aud4#3/F1) | CLOSED-by-a1f8f52 | (done: lexicographic-first) |
| C22 | ap-ir verify.rs:620-631, 1796-1813 | Name-resolution diagnostics: per-char String allocs, linear bound scans, near-quadratic on misses (aud4#5/F5) | IN-FLIGHT-g2 (`8e86c11` F5) | (in g2) allocation-free edit distance + sidecar indexes |
| C23 | threading.rs:178/185/348/791 | M1-M4: LOCALS/THREADS/PRIMS/OBJ_MIRROR insert-only, never evicted; pins whole payload graphs; THREADS also linear-scan per API call (aud5#1-4 = E1-E4; disclosed residual M1-M4) | IN-FLIGHT-g4 (`1b6955b`+`78c89a5`+`0be26d2`+`b9ee5f7` eviction sweep + O(1) indexes, wired at major-GC drain) | (in g4) shared `evict_stale_tables()` at collect drain |
| C24 | lxml.rs:2967-2978 | M11: XPATH_OBJECTS push-per-ctor, never removed; thread_local id-space vs worker threads = wrong-query aliasing (aud5#5; disclosed M11) | IN-FLIGHT-g4 (`1b6955b` self-carried XPath + process-global registry) | (in g4) |
| C25 | interp.rs:7518-7527 (frame exit) | Eager traceback wiring per unwound frame: 2 head-inserts O(depth²), 5 String allocs, deep clone of previous exception on EVERY raise, not just StopIteration (aud5#6 = A5; aud3-M11 classes.rs:42 tb clone = DUP-adjacent → M17) | CLOSED-by-963c9c7 (bare StopIteration arm); residual chain-clone + head-shift IN-FLIGHT-g1 (A5-tb-frames); classes.rs:42 tb_frames clone verified STILL PRESENT on main | Lazy tb materialize at render; innermost-first push + reverse |

### MEDIUM

| ID | File:line (live) | Defect | Status | Fix shape |
|---|---|---|---|---|
| M01 | interp.rs:7541-7557 (`scope_globals`) | Two fresh `HashSet<String>` built per call + per-param String/Rc alloc in bind_params (aud0#6) | IN-FLIGHT-g1 (A6, DeclaredNames Rc-shared) | Precomputed `Rc<HashSet<String>>` per scope |
| M02 | interp.rs:7418 (`plain_sentinel`) | Full TypeObject + fresh PyDict built per call for super() sentinel (aud0#7) | IN-FLIGHT-g1 (A6) | Two interned sentinel Rc shared |
| M03 | interp.rs:9162-9167 (`pull_iter`) | Read-borrow of `iterators` held across `it.next()`; re-entrant register/prune fails `try_borrow_mut` → 50µs sleep spin **forever** (same-thread cascade deadlock). Verified STILL PRESENT. Dead tail branch at ~:9188. | **OPEN** (highest-urgency open item — hang risk) | Clone the iterator handle, drop the borrow, then `next()` |
| M04 | interp.rs:8612+ (`run_comp`) | Snapshot (clone) of ALL enclosing frame locals per comprehension execution (aud0#9) | IN-FLIGHT-g1 (A6 parent-chain walk) | Walk parent frames by reference / capture only read names |
| M05 | iter.rs:117-131 | `iter(str/bytes/set)` materializes every element up front (10MB string = 10M tiny allocs per `for`) (aud0#10). Verified STILL PRESENT (Seq-collect arms). | **OPEN** | Index-cursor arms yielding on demand |
| M06 | interp.rs:33 `CELL_BACKING`; insert :11976 | Zero eviction; keyed by recycled Rc addresses → latent stale-hit correctness bug (aud0#11 = E8; G5 fixed the wrapper cache only) | IN-FLIGHT-g1 (E8, CELL_BACKING_PINS weak sweep) | Drive backing from same retain sweep / fold into wrapper |
| M07 | interp.rs:~225 `FILE_SRC` | Uncapped thread-local holding full file text per traceback file + full String clone per rendered frame line (aud0#12 = aud2-M1/L1 = disclosed **M10 residual**) | **OPEN** (disclosed) | `Rc<str>` values + FIFO/LRU cap ~64, or cache only split lines |
| M08 | gcmod.rs:439/480/571/652/664 | GC scratch Vec/HashMap rebuilt from zero per pass; YOUNG deep-cloned to build scope (aud1#6 = C6) | IN-FLIGHT-g3 (`0271294` reused scratch) | Thread-local scratch `clear()`-ed between passes |
| M09 | ap-obj dict.rs:254, value.rs:373 (clear), dict.rs:697 | Containers never shrink after prune: 1M-entry peak pinned forever; `PyList::clear` kept capacity (aud1#7 = C7) | IN-FLIGHT-g3 (`2148bfe` verified in commit body: shrink on pop, clear frees, empty set buckets deleted) | (in g3) |
| M10 | gcsupport.rs:296-313 | `fire_dead_weakrefs` builds full token index before the empty-check (aud1#10 = C8) | IN-FLIGHT-g3 (C8) | Peek-then-build |
| M11 | gcmod.rs:148-160 (`gc_register`) | Double hash probe per registration ×3-4 per object (aud1#9 = C9a) | IN-FLIGHT-g3 (`0271294` c9) — verified still contains_key+insert on main | Single entry-API probe |
| M12 | gcsupport.rs:1750 (`GilSendPub`) | `unsafe impl Send+Sync` is the soundness load-bearer for worker GC; hook fn-ptr transmute in gc_gate.rs:39 (aud1#11 = C9-safety) | **OPEN** (safety design item; gate for 02.09 no-GIL leaf) | Drop Sync impl / cfg-gate; typed AtomicPtr hook |
| M13 | importer.rs:182/352-397/1171/1789 | Path finder re-stats + re-canonicalizes vendored dir per lookup; no dir cache (aud2-M2 = B6) | **OPEN** | `OnceLock` vendored dir; per-base dir-entry cache |
| M14 | importer.rs:70 (`s()`) | Fresh GC PyString minted for every dict-key op in importer; feeds GC allocation triggers (aud2-M3) | **OPEN** (g3 adds `get_str`/interned helpers it can adopt) | Use `PyDict::get_str` + interned magic names |
| M15 | importer.rs:1337; interp.rs MODULE_FRAME_CELLS:14113 | Never pruned; keyed by raw frame pointer → recycled-addr aliasing across modules (aud2-M5). g4 does NOT cover it (frame.rs/interp.rs is g1's slice; main verified ptr-keyed). | IN-FLIGHT-g1 (`d016c21`+`cf39f47` Frame::drop retire by monotone id) | (in g1) |
| M16 | importer.rs:566 `saved_super` | SuperStackGuard clones whole super_stack BEFORE the sys.modules fast path — per-executed-import O(depth) churn (aud2-M6). Verified STILL PRESENT. | **OPEN** | Build guard after the cache hit, or snapshot length only (restore = truncate) |
| M17 | classes.rs:42 (`traceback_value`) | tb_frames Vec fully cloned (String pairs) per exception materialization (aud3-M11) | IN-FLIGHT-g1 (A5-tb-frames) — verified still present on main | Iterate under borrow; snapshot only needed range |
| M18 | ap-parse lowering.rs:1246/205/288/1666 | Comprehension clause clone-discarded; linear dedupe + per-Name String alloc; whole bound-map cloned per module (aud2-M7 = B7) | IN-FLIGHT-g2 (`8e86c11` B7) | Return owned iterable; HashSet sidecars; mem::take + early return |
| M19 | builtins_modules.rs pystr arms | Receiver String copied before use (14+ sites) (aud3-M1 = D9) | CLOSED-by-529ad9e/f2fef14 — needs re-verify: 33 `.as_str().to_string()` hits remain in file (some cold, some live) | Audit the residual 33 sites |
| M20 | methods.rs:17, builtins/classes/builtins_modules many sites | `std::env::var` probes per-op (aud0#4, aud1#12, aud2-L3, aud3-M2 — 4 auditors) | CLOSED-by-2b8f41c (AtomicU64 bitset) — verified live: methods/classes/builtins now call `debug_flag()`; interp.rs down to 1 hit. Residual: `ap_trace` per-import `env::var_os` → L02 | (done) |
| M21 | builtins_modules.rs len arms | `chars().count()` instead of cached `PyString::len()` (aud3-M3) | CLOSED-by-529ad9e (cached str len) — verified live: residual `chars().count()` only at :1128/:1155/:2222 (encode_utf16/32 paths where count is the semantics) — needs re-verify those arms | Use `s.len()` |
| M22 | builtins_modules.rs casefold/swapcase | `Vec<char>` alloc per char (aud3-M4) | CLOSED-by-f2fef14 — needs re-verify (pattern `to_lowercase().collect::<Vec` no longer greps on live) | (assumed done) |
| M23 | methods.rs str.join | Three buffers materialized (aud3-M5) | CLOSED-by-f2fef14 — verified live (pre-sized comment at methods.rs:1388) | (done) |
| M24 | builtins.rs print | Output String cloned into redirect path (aud3-M6) | CLOSED-by-0858b53/f2fef14 — needs re-verify (builtins.rs:225 shape changed; no `out.clone()` grep hit) | (assumed done) |
| M25 | methods.rs partition | Results computed twice, first pass discarded (aud3-M7) | CLOSED-by-f2fef14 — needs re-verify (`let _ = (head, tail)` no longer greps) | (assumed done) |
| M26 | format.rs pad paths | Multi-alloc `format!` chains per f-string field (aud3-M8) | CLOSED-by-f2fef14 — verified live (`with_capacity` writers at format.rs:190/302/311/320) | (done) |
| M27 | builtins_modules.rs json.loads | Double byte-payload copies (aud3-M9) | CLOSED-by-f2fef14 — needs re-verify (`from_utf8((**b)` no longer greps) | (assumed done) |
| M28 | builtins_modules.rs b16encode | String alloc per byte (aud3-M10) | CLOSED-by-529ad9e — verified live (HEX16 nibble table at :11742 with audit comment) | (done) |
| M29 | builtins_modules.rs:13302 lru_cache | String-encoded key + O(cache) `order.position/remove` per call (aud3-M12). Verified STILL PRESENT. | **OPEN** | HashMap key→idx + VecDeque for O(1) LRU refresh |
| M30 | builtins_modules.rs pybytes join/replace | No with_capacity, double materialization (aud3-M13) | CLOSED-by-f2fef14 — verified live (D9 borrow-parts comment at :17141) | (done) |
| M31 | builtins_modules.rs:2416 re_sub_literal | Whole text into Vec<char> per sub (aud3-M14) | CLOSED-by-f2fef14 — verified live (&str slice version with D9 comment) | (done) |
| M32 | builtins_modules.rs operator arms | `two()?` (clone helper) called twice per op (aud3-M15) | CLOSED-by-f2fef14 — verified live (`let (a, b) = two()?` destructures at :4012+) | (done) |
| M33 | builtins_modules.rs:15352 | dict-subclass re-point allocated + recursed per call (aud3-M16) | CLOSED-by-f2fef14 — verified live (`recv_override` local) | (done) |
| M34 | ap-host lib.rs:50-59 | wasmtime Engine rebuilt + JIT per call (aud4#6/F2) | CLOSED-by-0eff19f (engine reuse) | (done) |
| M35 | ap-cli main.rs:171 merge_dirs | `add` copy-walked staged tree instead of rename (aud4#7/F3) | CLOSED-by-0eff19f (rename-merge) | (done) |
| M36 | generators.rs:545 `make_fiber` | Fresh mmap+munmap per generator, no pool; 8MB VA per suspended fiber (aud5#7 = E6) | IN-FLIGHT-g4 (`1b6955b` size-classed free list + `4101ea5` munmap on thread exit) | (in g4). Residual: `thread_start` still `.stack_size(64 << 20)` on wave3-merged:882 → M40 |
| M37 | threading.rs:686/610/853/663 | Poll loops where condvars exist: join_wait 200µs poll, 2000×50µs identity stamp, attr retry loops, global-Mutex cancel poll (aud5#8 = E7) | IN-FLIGHT-g4 (`1b6955b`+`78c89a5`: JOIN_CV awaited, flag-marked done, bounded stamp, per-addr Condvar) | (in g4). Residual: current_os_tid locks TIDMAP per op → L06 |
| M38 | lxml.rs:1537 `register_doc` | O(n) `Weak::upgrade` slot scan per parse (aud5#9 = E9). Verified STILL PRESENT on main. | IN-FLIGHT-g4 (`1b6955b`: O(1) freed-slot + harvest list) | (in g4) |
| M39 | gcmod.rs:652 / value.rs:52 / dict.rs fmt_repr | (aud1#14) Value-size doc claim — the AUDIT'S CLAIM IS WRONG: niche packing keeps `Value` at 16 bytes; g3 `2148bfe` measured it, fixed the doc, and added a size_of pin test. Placeholder line deleted too. | IN-FLIGHT-g3 (doc correction) | (in g3) |
| M40 | threading.rs:564 (live main; :882 on wave3-merged) | 64MB OS stack reservation per Python thread, no pool — pre-F1 size, missed by g4 | **OPEN** | Cap to a sane default (8-16MB) + env override |
| M41 | weakbridge.rs:146-162 | CollectGuard inner Drop clears IN_COLLECT early (nesting footgun) (aud1#15) | IN-FLIGHT-g3 — verified: wave3-merged converted it to a `Cell<u32>` depth counter | (in g3) |

### LOW / disclosed residuals

| ID | File:line | Defect | Status | Fix shape |
|---|---|---|---|---|
| L01 | interp.rs by_path inserts :332/:423 | Last-writer-wins orphans resident rows; ids never reused (aud2-L2) | **OPEN** | Tombstone-free compaction on registry rebuild |
| L02 | importer.rs:41-45 `ap_trace` | `format!` message built BEFORE the env gate (aud2-L4) | **OPEN** | Gate on `debug_flag`-cached bool; pass fmt closure |
| L03 | importer.rs:228-236 `sys_modules_value` | Full-snapshot copy API (aud2-L5) | **OPEN** (benign: startup-only) | Return borrow or iterator |
| L04 | gcmod.rs:1139 `weakref_prune` | Dead no-op fn (aud1#13). Verified still present on main (live variant returns nothing / retain-closure dead; real prune is `weakref_prune_fired`). | **OPEN** | Delete |
| L05 | methods.rs dict views | View construction still snapshots pairs post-D5 (C16 residual) | **OPEN** | Hold live dict Rc, iterate under borrow |
| L06 | threading.rs:77 `current_os_tid` | Global TIDMAP Mutex lock on every GIL acquire/release, local get/set (aud5#8 residual) | **OPEN** | `thread_local! Cell<u64>` cache |
| L07 | ap-ir dump.rs:940+ | String-per-node + Vec+join on golden-dump critical path (aud4#8/F4) | **OPEN** | Writer-form `&mut String` (deterministic output preserved) |
| L08 | ap-ir verify.rs:2640+ fold | Digit-string round-trips in constant folding (aud4#9) | **OPEN** | i128 fast path; write! into pre-sized buffer |
| L09 | ap-run style cluster | aud3 L1-L6 + aud0#14 (with_capacity misses, dead branches, re-lookup of `__anext__` per step, sleep-spin loops without escalation) | MIXED: dead-code arms removed by f2fef14 (methods.rs:1613 comment); the rest **OPEN** | Sweep commit |
| L10 | interp.rs:7865 | Disclosed **M12** PENDING_GEN_ERROR / PENDING_GEN_ERROR_DEFAULT: uncapped thread-local, drains one-at-a-time | **OPEN** (disclosed) | Bound + drain-on-consume with cap |
| L11 | ap-obj classes cell storage | Disclosed residual: closure-carrying nested defs ~3MB/1k hot-loop calls (per-object cells; filed in AGENTS status) | **OPEN** (disclosed) | Shared cell arena per code object |
| L12 | frame.rs:183/217/227/257/262 + interp.rs:2749+ | Identifier reads mint fresh `Gc<PyString>` in the INTERP layer (aud0#5 = A4-run-side; aud1#8 value layer = g3 `2148bfe`) | Value layer IN-FLIGHT-g3; **frame.rs/interp.rs sites OPEN** (verified still minting on main; g3 does not touch frame.rs) | Adopt g3's `get_str`/`interned_str_ref` at frame.rs + `lookup_name` call sites after merge |

**Verified-clean list (auditors checked; carried forward, no row):** NESTED_DEDUPE
FIFO cap, iterator-registry prune (0858b53), exhausted-gen retire (G8), stdout
drain (G7), RE_CACHE/SRE_TABLE_CAP, rss_guard, structmod, exc_hier.is_subclass,
ast_bridge BRIDGE_SOURCE, blocking primitive waits (GIL released correctly).

---

## 2. Wave plan — OPEN items only (severity desc, blast-radius asc)

Merge order assumption: g2+g3+g4 land from `wave3-merged` first; g1 needs a
full gate run before trust. Items below do not collide with in-flight stacks.

| # | Item | Est. min | Notes |
|---|---|---|---|
| 1 | L02 ap_trace gate-before-format | 10 | one function, cold-ish path |
| 2 | L04 delete dead weakref_prune | 10 | grep call sites first |
| 3 | M16 SuperStackGuard after cache-hit | 15 | importer.rs only; restore-truncate shape |
| 4 | M40 cap thread stack 64MB | 15 | one line + env; gate: threading suites |
| 5 | **M03 pull_iter borrow-across-next (hang risk)** | 20 | 3-line clone-then-drop; generator-in-loop corpus case as the test |
| 6 | L06 current_os_tid TLS cache | 20 | threading.rs only |
| 7 | M07 FILE_SRC cap + Rc\<str\> (disclosed M10) | 30 | interp.rs; LRU-64 + refcount clone |
| 8 | M05 iter() cursor arms for str/bytes/set | 40 | iter.rs + ~4 call sites; byte-exact corpus |
| 9 | M29 lru_cache O(1) | 45 | builtins_modules arm; functools corpus case |
| 10 | M13 path-finder stat/dir cache + OnceLock | 60 | importer.rs; measure scrapy I0-I8 time before/after |
| 11 | M14 importer s() → get_str/interned | 30 | after g3 merge (helpers exist on branch) |
| 12 | L12 frame.rs/interp.rs name interning | 60-90 | after g3 merge; LOAD_GLOBAL-heavy probe; byte-exact gate |
| 13 | C05 PyObject::new allocation diet | 120 | ap-obj core: lazy attrs/dunders, single-probe register; full gc+corpus gate |
| 14 | L05 dict view live-backref | 45 | after C05/g3 settle |
| 15 | L07 golden-dump writer-form | 60 | byte-exact golden gate must stay green |
| 16 | L08 fold i128 fast path | 60 | ap-ir; golden dump gate |
| 17 | L09+L01 low sweep (reserves, dead tail, by_path compaction) | 60 | combined cleanup commit |
| 18 | L12-disclosed pair: M12 PENDING_GEN_ERROR bound (L10) + closure-cell arena (L11) | design pass first | both disclosed residuals; scope after g1/g4 merge stabilizes |
| 19 | M12 GilSendPub soundness | defer | fold into 02.09 no-GIL leaf design |

Deferred pending merge verification: re-check rows tagged `needs re-verify`
(M22/M24/M25/M27/M19-residual) once `wave3-merged` lands, and confirm C20's
live-vs-test-path conflict before re-flagging ap-ir verify as a hot-path issue.

---

## 3. Spot-verification log (live tree, 2026-11-07)

Auditors ran at `9662de2`; main moved to `f2fef14`. Rows verified by reading
the live file:

| Row | Claim | Live result |
|---|---|---|
| C01 | interp.rs:2052 note_py_loc | FOUND at interp.rs:2066 (drift +14 lines) — defect unchanged: script_path.clone() + last_executed_line per call |
| C06 | O(heap) scan per ref() | FIXED — gcsupport.rs:252 calls `ap_gc::get_object_by_addr` (73fe13b) |
| C09 | died_now.contains | FOUND gcmod.rs:942-947, unchanged |
| C10 | lib.rs:106 naive offset_to_line | FOUND unchanged — g2 fixes it on branch only |
| C20 | verify.rs:1748 child_exprs clone | FOUND unchanged on main — g2 fixes on branch; aud2-vs-aud4 live-path conflict noted |
| M03 | pull_iter borrow across next | FOUND interp.rs:9162-9167, unchanged |
| M05 | iter.rs eager collect | FOUND iter.rs:117-131, unchanged |
| M06 | CELL_BACKING | FOUND interp.rs:33, insert-only (no remove/retain anywhere) |
| M01/M02 | scope_globals 7483 / sentinel 7384 | drift: now 7541 / 7418; both defects unchanged |
| M16 | saved_super 564 | FOUND at 566, still cloned pre-fast-path |
| M17 | classes.rs:42 tb clone | FOUND unchanged |
| M29 | lru order.position | FOUND at :13302 (drift from 13059) |
| M20 | env::var hot sites | CLOSED confirmed: methods.rs/classes.rs/builtins.rs use `debug_flag()` (0 grep hits for env::var in methods/classes/builtins); interp.rs 1 hit; importer 3 (startup + ap_trace) |
| M28 | b16encode | CLOSED confirmed at live :11742 with audit-naming comment |
| M18/C10-adj | offset_to_line naive | confirmed present (g2 in-flight) |
| C23-C24 | threading/xpath | main unchanged (expected — g4 on branch); M40 residual verified: wave3-merged STILL `.stack_size(64 << 20)` at threading.rs:882 — 64MB not capped by g4 |
| M41 | IN_COLLECT bool | wave3-merged weakbridge.rs is depth counter — g3 fix confirmed |
| M39 | Value ≥24 bytes claim | DISPUTED and resolved by g3: niche packing keeps 16 bytes; doc fixed + test pinned |
| C12-C33 (aud3 set) | spot-checked C18/C19/C23-shape/M23/M26/M28/M30/M31/M32/M33 | CLOSED confirmed via D-series comments + changed code shapes |

Line drift is endemic (audit-era citations sit ~10-400 lines below live
positions); statuses above cite the LIVE lines.
