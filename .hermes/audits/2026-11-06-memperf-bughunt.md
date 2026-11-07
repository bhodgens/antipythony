# Memory & Performance Bughunt — 2026-11-06 (wave 3 / memperf)

Scope: all 12 crates, six parallel read-only auditors (interp hot loop;
object model + GC; import machinery; builtins/methods; codegen + packaging;
threading/fiber/lxml). Base tree: 9662de2 + staged sibling edits
(ast_bridge.rs formatting, classes.rs kwargs fix). Every cited line ref was
spot-reverified by the parent against the live tree. Check-against rules:
rust-best-practices §12 + rust-pro memory/ownership guidance.

Prior campaigns deliberately EXCLUDED (not re-reported as new): OOM campaign
#1 (docs/import-leak-recheck-2026-11-05.md) and #2
(docs/oom-fix-verification-2026-11-06.md, 15 findings closed). Known open
residuals from campaign #2 were characterized, not re-discovered: M1-M4
threading tables, M10 FILE_SRC, M11 XPATH_OBJECTS, M12 PENDING_GEN_ERROR,
closure-cell ~3MB/1k.

## Gate baseline (parent-run, before any fix)

- `cargo test --workspace -j 4`: 4 suites green (0 failed at that point),
  then `ap-cli::ast_getattr_fence::ast_getattr_leak_fence` FAILED — fence
  killed its probe at the 600s deadline while six auditors saturated CPU.
  Suspected contention artifact. MUST re-run solo before treating as a
  regression (it is the campaign-#1 regression guard).
- pytest / corpus / T1 gates: not yet re-measured this wave.

---

## CLASS A — per-op taxes on the eval loop (fix first; every program pays these)

**A1. HIGH — `note_py_loc` clones a String + scans line tables on EVERY
expression node.** `interp.rs:2865` (eval_expr entry) → `interp.rs:2051-2071`:
`interp.script_path.clone()` (heap String) + `last_executed_line` with a
try_borrow/sleep(50µs) retry loop, per AST node. `x + y` = 3 invocations.
Fix: store a raw pointer/`&'static str` handle + line in a Cell; or only
record at diagnostic-consumption sites.

**A2. HIGH — deep-clone of the full function AST twice per call.**
`frame.rs:515-525` `fn_entry()` returns `.cloned()` FnEntry (owns whole
`FnDef` body tree); `interp.rs:7253-7257` clones `entry.def` again.
Fix: registry stores `Rc<FnEntry>`; read `&entry.def`.

**A3. HIGH — debug env-var probes on hot dispatch paths (≈90 sites).**
`std::env::var` count: 58 in interp.rs alone (every `raise()` :52, every
`eval_binop` :4162, every `attr_get` :11186/:11201 — the `_rawentry` arm is
NOT env-gated and can run `Backtrace::force_capture()`; every call :4620),
plus `methods.rs:16` (inside raise — every exception anywhere), 8 classes.rs
sites, builtins.rs len/__str__, importer.rs, `gcmod.rs:536/722` (twice per
collect). getenv = environ scan + String alloc per op.
Fix: one `static APX_FLAGS: OnceLock<DebugFlags>` built at entry; all sites
read cached bools. Same pass: gate the `_rawentry` force_capture behind the
flag.

**A4. HIGH — no identifier interning at the value layer: fresh
`Gc<PyString>` minted per name read / attr probe / dict op.**
`frame.rs:217/227` (lookup class_ns + globals), `interp.rs:2749/2761`
(builtins fallback), `frame.rs:183/257/262`; `ap-obj/classes.rs:109/120`
(one PyString alloc + full hash PER MRO entry per attribute lookup;
`__del__` probe pays it per garbage instance per collect);
`dict.rs:414-417` `values_eq` → `weakref_target_addr` allocates
`PyString::new("<target_addr>")` on every Obj/Obj dict probe;
`importer.rs:70-72` `s()` per key op; magic attr names re-created per call
(gcsupport.rs:277/1076/1129). A LOAD_GLOBAL miss chain costs 4 GC string
allocs + 4 hashes. `interned_type_pub` fixed `_ast` only.
Fix: process-global interned consts for the ~30 magic/dunder names + a name
table so `Expr::Name`/attr reads carry one handle; `PyDict::get_str(&str)`
fast path; `contains_probe` without value clone (dict.rs:249-251).

**A5. HIGH — per-frame exit eager traceback: StopIteration in every hot loop
pays O(depth²) insert(0) + 5 String allocs + recursive exception clone.**
`interp.rs:7444-7468`: `exc.tb.insert(0, ...)` + `tb_frames.insert(0, ...)`
(head-shift per frame per unwind) + `(*prev).clone()` deep-clone of prior
exception incl. full tb vectors (exc_hier.rs:146-170). 14 StopIteration
raise sites; `iter.rs:309-311` mints a fresh class String per exhaustion.
Caught-exception frames are discarded (interp.rs:1113/1168/1800) = pure
waste.
Fix: record innermost-first, reverse at render; materialize `exc.tb` lazily
from the snapshot range; special-case bare StopIteration (no tb walk).

**A6. MED — per-call waste cluster in function-call setup.**
`interp.rs:7336-7337→7483-7499` two fresh `HashSet<String>` per call
(scope globals/nonlocals); `interp.rs:7384-7397` a full `TypeObject` +
fresh PyDict built per call as the super() sentinel; `interp.rs:7521-7523`
per-param String + Rc<RefCell> allocs; `interp.rs:8566-8576` comprehension
snapshots ALL enclosing locals per execution (ScopeInfo knows the reads).
Fix: precompute sets per scope in registry; two interned sentinel Rc's;
capture only comp-read names (clone the Rc slot handle, not the value).

## CLASS B — quadratic import machinery (fixes the 70–120s scrapy chain, part 2)

**B1. HIGH — `extend_current` rebuilds the whole FnRegistry + merged scope
tables per import: O(M²) deep AST clones + permanently leaked merged
tables.** `interp.rs:414` `Box::leak(Box::new(scopes))`; `:418-419`
`cur.entries.clone()` + `cur.by_path.clone()` (every prior module's full
function bodies re-copied); `merge_scopes_static` `:446-490` deep-clones
every accumulated scope into a new BTreeMap, leaks the union, orphans the
old one. ~270 modules/process ⇒ quadratic time + GC-immune growth (the
importer's own comment at `importer.rs:671` names this cost).
Fix: long-lived `Rc<RefCell<Vec<FnEntry>>>` + `HashMap<String,u32>` mutated
in place (ids are append-only indices, interp.rs:401-402); scope tables in
one process-lifetime map keyed (module-id, path), no per-import union/leak.

**B2. HIGH — `collect_stmt_lines` quadratic line lookup, second un-fixed
copy.** `ap-parse/src/lib.rs:106-112` `offset_to_line` = newline count over
`source[..off]` per statement — the identical bug the Lowerer already fixed
(lowering.rs:230-234 comment). Runs per import.
Fix: line_starts table + binary search (reuse `Lowerer::line_at`,
lowering.rs:590-594).

**B3. HIGH — ap-ir verifier O(n²) subtree clone walk per module.**
`verify.rs:1748-1790` `child_exprs(&Expr) -> Vec<Expr>` deep-clones every
child; caller `:901-904` recurses holding each frame's clone Vec — peak
live O(n²) on chained expressions, per imported module.
`verify.rs:176-178/665-667/717-719` clone the entire enclosing-scope chain
(all ancestor `Vec<String>` + module bindings) per def item.
Fix: `child_expr_refs` iterator (fold_child_exprs at :2022 already walks
&mut); `Rc<EnclosingLeg>` shared legs.

**B4. HIGH — no cross-process compile cache; the ap-pack cache is
write-only.** `importer.rs:439-458` compile_file re-reads/re-parses/
re-lowers; zero `.pyc`/IR cache (grep clean). `ap-cli/src/main.rs:327-339`
cmd_run computes the SHA-256 cache key, writes the marker, then ignores it
(`let _ = cache_entry;`) and re-parses. Same at main.rs:414-421.
Fix: serialize lowered `ap_ir::Module` + `ScopeInfoModule` (postcard/
rmp-serde) keyed (path, mtime, size) under the existing entry dir; skip
parse+lower on hit. Pairs with leaf 02.06 (AoT replaces the payload later).

**B5. MED — scope-path keying collisions + per-miss O(scopes) joins.**
Bare dotted-name keys merge across modules (`wrapper` in A vs B collides;
"richest-wins" heuristic interp.rs:368-387 picks arbitrary winner — the
twisted deprecate.py NameError at interp.rs:450-453 is a production
instance); `scope_of` miss path does `s.path.join(".")` fresh String per
scope (interp.rs:369-371 / 349-391) with no negative caching; lambda/
LocalDef eval call it per execution (interp.rs:3346/1521).
Fix: key (module-id, path); precomputed joined key + HashMap index;
cache misses.

**B6. MED — path finder re-stats/re-scans per lookup.**
`importer.rs:352-397` fresh `Vec<String>` sys_path per miss (`:219`
String churn per entry), 2 syscalls per base; `vendored_stdlib_dir()`
re-`canonicalize()`s per call (:182, invoked inside find_on_path :363);
attach_native_vendored_path :1171-1183 + augment_with_vendored :1789-1794
rescan all entries. `SuperStackGuard` (importer.rs:564-569) clones the whole
super_stack BEFORE the sys.modules fast path (:627-629).
Fix: OnceLock vendored dir; dir-listing cache keyed mtime; move guard
construction after the cache-hit check (or snapshot length only).

**B7. MED — lowering hot-loop waste.** `lowering.rs:1246` pushes a
`gens.to_vec()` deep clone that `:1259` immediately discards; `:205-209`
linear `any()` dedupe + per-occurrence `to_string()`; `:288`
`finished_bounds` full clone per module even when no deferred frees;
`:1666-1673` alias path allocated then re-segmented.

## CLASS C — GC/object-model per-cycle & per-allocation costs

**C1. HIGH — weakref deref does a FULL-HEAP clone-scan per call.**
`gcsupport.rs:247-251` `target_value_by_addr` = `ap_gc::get_objects()`
(Rc inc per registry entry + full Vec, gcmod.rs:1010-1012) then linear
find. Called per `ref()`/`weakmethod`/`finalize_detach` and per
WeakKeyDictionary `keys()/items()/pop()` entry → O(N × heap).
Fix: `get_object_by_addr` reading `REGISTRY.get(&addr)` — the registry is
already keyed by exactly this address. O(1).

**C2. HIGH — `edges_of` deep-clones every traced container during the GC
scan.** `gcmod.rs:332` `attrs_guard.clone()` copies the whole `__dict__`
per instance per pass, then `.items()` clones every pair AGAIN (:333,
dict.rs:292-299); `:291` Dict snapshot; `:301-302` Set snapshot
(PySet::iter = elements_snapshot clone, dict.rs:683-685). `edges_of` only
pushes usizes — no re-entrancy — snapshots are pure waste, ×2 passes.
Fix: borrow-guarded `items_ref()` iteration yielding `(&Value,&Value)`;
delete the clone line.

**C3. HIGH — root seeds re-snapshotted for the whole heap every collect,
gen-0 included.** `gcmod.rs:513-535` ROOT_DICTS/ROOT_MODULE_NS `.items()`
clone-all + `sys_modules_values()` clone-every-module
(importer.rs:250-252) per fire (~every 700 allocs); `:677` FROZEN clone per
pass (after `gc.freeze()` that's an O(heap) HashSet copy); `:751`
FINALIZED_ADDRS clone.
Fix: persistent seed `Vec<usize>` invalidated only on root add/remove;
borrow FROZEN/FINALIZED instead of cloning.

**C4. HIGH — every `PyObject::new` ≈ 4 allocations + 3-4 GC registrations
(~12 hash inserts).** `ap-obj/src/object.rs:124-151` (+dup :162-187):
eager `PyDict::new()` attrs, dunders built as a throwaway `Gc<PyDict>` then
deep-cloned (:138), then 3 registry maps per registration
(gcmod.rs:149-159) with a double probe (:149-155).
Fix: lazy `Option<Gc<PyDict>>` attrs (CPython-style NULL dict); dunders
only for UserClass; single-probe entry API; slab/arena (fits MMTk-swap doc).

**C5. MED — sweep_weakrefs O(WEAKREFS × GARBAGE_NOW).** `gcmod.rs:942-951`
`died_now.contains()` linear Vec scan per weakref entry + Vec clone per
pass. scrapy's LocalWeakReferencedCache pattern hits this directly.
Fix: HashSet seed (or store GARBAGE_NOW as a set).

**C6. MED — GC scratch never reused between passes.** `gcmod.rs:439-440`
fresh `Vec::new()` kids per traced object; `:571-572` same mark-walk;
`:664` counts map rebuilt per collect; `:480-481` visited/queue fresh;
`:652` YOUNG full clone to build scope.
Fix: thread-local scratch buffers `clear()`ed (retains buckets) between
passes; `iter().copied().collect()` into reusable buffer.

**C7. MED — collections never shrink after prune.** `dict.rs:254-264` pop
leaves holes, resize never shrinks (1M-entry peak retained forever);
`value.rs:373-376` `PyList::clear()` keeps capacity while `allocated.set(0)`
lies (next append won't reserve — CPython frees); `dict.rs:697-711` set
buckets stay as empty Vecs in a never-shrinking HashMap. Also: PySet
`HashMap<u64, Vec<Value>>` allocates a Vec per distinct hash — the compact
PyDict layout would be smaller AND faster.
Fix: rehash-to-shrink at `used*8 < len`; clear drops the backing Vec;
delete empty buckets.

**C8. MED — `fire_dead_weakrefs` builds a full token index BEFORE the
empty-check.** `gcsupport.rs:296-313`: snapshot of every live token (Rc
clone each) runs first; `dead.is_empty()` early-return comes after — pure
waste on every no-death collect. `weakref_is_dead`/`weakref_count_for`
(gcmod.rs:1112-1125/1035-1042) linear-scan WEAKREFS per query.
Fix: take_dead first, build index lazily; side target→tokens map.

**C9. SAFETY-FLAG — `unsafe impl Send+Sync for GilSendPub<T>` (gcsupport.rs:
1750-1752) is the sole thing preventing UB under worker threads;
`gc_gate.rs:39` fn-ptr transmute.** Not a perf bug; the hook swap at
:1796-1803 forwards main-thread registrations with no lock over
`gc_register`'s thread-locals. Gate/typed-AtomicPtr before 02.09 no-GIL work.

**C10. LOW — `Value` size doc wrong** (value.rs:52 claims 16 bytes; Int arm
embeds a 16-byte enum ⇒ ≥24; real cost per stack slot). Dead placeholder
`Rc::new(0u8)` dict.rs:318-319. `weakref_prune` no-op-always-true
(gcmod.rs:1128-1140) delete or wire. `APX_GC_DUMP` getenv ×2 per collect.
CollectGuard nesting footgun (weakbridge.rs:146-162) — depth-counter when
MMTk lands.

## CLASS D — stdlib method layer (per-call waste on library hot paths)

**D1. HIGH — bytearray: every mutating op round-trips the whole buffer 3×
+ rebuilds repr.** `builtins_modules.rs:5900/5916/5992` — `bytearray_state`
full clone in, `bytearray_set` clone out + `bytes_repr` full scan. `ba.append`
loop = O(n²), ~3 full copies/op. Exact class of StringIO fix 72b6fb5.
Fix: `Gc<RefCell<Vec<u8>>>` `<data>` slot mutated in place; lazy repr.

**D2. HIGH — pylist mutators rebuild the whole list per op.** `to_vec` +
`clear` + re-append loop: insert :16150, pop :16176, remove :16204, reverse
:16249, sort tail-rewrite :16066/:16101; `methods.rs:1641-1658`.
`lst.insert(0,x)` loop = O(n²). PyList already has remove_range/set_at
(value.rs:356-366).
Fix: in-place `insert_at/remove_one/reverse_in_place` under one borrow.

**D3. HIGH — `s[i]` single-char string subscript is O(n).**
`methods.rs:173-177` materializes `Vec<char>` per access; slice :183-184
builds 3 materializations. `for i in range(len(s)): s[i]` = O(n²).
Fix: ASCII fast path (byte index — `is_ascii` cached on PyString);
char_indices walk otherwise.

**D4. HIGH — singledispatch clones the entire impl registry per dispatch.**
`builtins_modules.rs:4713-4717` `r.impls.clone()` (full HashMap) per
`_sd_<id>(x)` call; used by pathlib/json/pytest plugins.
Fix: choose inside the borrow, clone only the selected Value.

**D5. HIGH — dict method layer materializes the whole dict per call.**
`ap-obj/dict.rs:292-303` `items()`/`keys()` pair-clone everything; pydict
copy :15193, views methods.rs:1698-1727, maketrans :17112. Loops calling
`.update()` = O(n²) with 2n clones/op.
Fix: `foreach(&self, f)` under one borrow; views hold the live dict Rc.

**D6. MED — strip/translate/bytes-translate: `set.contains(char)` per
character (O(n·m)).** `builtins_modules.rs:15528-15543/16485-16496`,
`methods.rs:1552-1567`. Fix: 256-bit bitmap (CPython's method).

**D7. MED — textwrap.wrap re-counts the line per word.**
`builtins_modules.rs:5605-5611` `cur.chars().count()` per word → O(chars×
words). Fix: running counter.

**D8. MED — IncrementalDecoder clones the whole buffer per chunk.**
`builtins_modules.rs:1211` `String::from_utf8(acc.clone())` per chunk →
O(n·k). Fix: `from_utf8(&acc)` + drain valid prefix, buffer only the
incomplete tail (CPython shape).

**D9. MED — str surface copies the receiver String before use (14 sites),
`chars().count()` despite cached `len()` (len/justify/center/zfill/
__len__), casefold per-char Vec allocs, join 3-buffer materialization,
print clone into redirect, partition double-pass, format.rs multi-alloc
padding, json.loads double byte copy, b16encode String-per-byte
(`hex_nibbles` already exists :12285), lru_cache O(cache) key scan +
string-encoded keys, pybytes join/replace no with_capacity, re_sub_literal
2 full Vec<char> copies, operator `two()` called twice per comparison,
pydict-subclass re-dispatch fresh Vec per call, traceback_value() clones
frame Vec per exception materialization (classes.rs:42).
(file:line detail in auditor report, delegated transcript task-3.log.)
Fix shapes are all mechanical: borrow the receiver, use cached len,
extend iterators, reserve totals, single-pass.

## CLASS E — threading/fiber infrastructure (characterizes M1-M4, M11)

**E1-E4. HIGH — M1 LOCALS (threading.rs:185), M2 THREADS (:178), M3
OBJ_MIRROR (:791), M4 PRIMS (:348): all four process-lifetime tables are
insert-only; parent-verified zero removal sites.** Per-(local×thread)
PyDict pin (M1), full dead-thread object graphs pinned + O(cumulative)
linear scans under a global Mutex on every threading API call (M2),
every-constructed-Thread/Timer pinned incl. target/args payloads + addr
aliasing (M3), every Lock/Queue primitive pinned with strong
`QueuePrim.items` (M4). CANCELLED (:195) and TIDMAP (:81) same.
Key fix lever: ONE shared eviction sweep at the existing major-GC drain
hook (gcsupport.rs:638, the only removal path in the tree) fixes all four
plus the two side tables. Weak mirrors (`Weak<PyObject>` + upgrade) is the
per-table alternative.

**E5. HIGH — M11 XPATH_OBJECTS (lxml.rs:2967-2978): push per
`etree.XPath()` ctor, never removed; index-keyed so naive removal
corrupts live markers (why it stayed open).** Scrapy/parsel re-construct
per parse: 10k-page crawl ⇒ ~100k entries + every passed namespaces dict
pinned forever. SECOND bug same site: thread_local table + shared
interpreter across workers ⇒ cross-thread id reads (returns None or the
wrong query on length collision). Class-of bug with DOCS/RE_CACHE
thread-locals — flag for 02.09.
Fix: store query in the marker's own attrs (`call_xpath_object` :2990
already reads `<query>` — the registry exists only because marker dispatch
lacks self-access; give markers a self-carrier, delete the table).

**E6. MED — fibers: fresh mmap+munmap per generator, no pool.**
generators.rs:545-566 `DefaultStack::new` = mmap + 2 guard pages per
def-generator creation (corosensei-0.3.4 unix.rs:25-76; no free-list in
0.3.4, none implemented). 8 MB VA held per SUSPENDED fiber (F1 floor);
1000 suspended = 8 GB VA. Thread side: `.stack_size(64<<20)` per OS thread
(threading.rs:562-565) — the pre-F1 size, unpooled.
Fix: size-classed thread-local stack free-list implementing corosensei's
`Stack` trait; revisit 64 MB OS stacks.

**E7. MED — poll loops where condvars exist.** `join_wait` (:686-705) polls
200µs while JOIN_CV/JOIN_MU are notified (:632/:658) and never awaited;
worker_bootstrap identity retry 2000×50µs; attr_str/attr_bool 2000×50µs
tries (called 3× per thread start); `wait_cancellable` (:663-675) locks the
global CANCELLED mutex once per 1 ms slice (60 s sleep = 60k locks);
`current_os_tid()` locks global TIDMAP on every acquire_gil/local op —
belongs in a `Cell<u64>` thread-local.
Fix: await JOIN_CV with a done-predicate; per-addr Condvar or AtomicBool
cancel flag; cached tid.

**E8. MED-LOW — new: CELL_BACKING has zero eviction (campaign #2's G5 fixed
the WRAPPER cache only).** `interp.rs:33` TLS `HashMap<usize, CellSlot>`;
insert-only :11976, no remove anywhere (parent-verified); wrapper retain
sweep :11955 drops cache entries but not backing entries; ptr-recycled keys
= stale-hit correctness hazard (a re-minted wrapper inherits an old slot).
Fix: fold backing onto the wrapper object (side enum on Namespace payload)
or drive both maps from one sweep.

**E9. LOW — lxml register_doc O(n) Weak::upgrade slot scan per parse**
(lxml.rs:1537-1563, introduced by G3 fix); freed-slot list already
anticipated by the code's own comment :1541.

## CLASS F — codegen/packaging determinism + compile memory

**F1. HIGH (contract-level) — purity check emission order is
filesystem-dependent.** `ap-pack/src/purity.rs:45-68`: `read_dir` order +
LIFO stack + early-return ⇒ with multiple native wheels (scrapy pulls
lxml+brotli+cryptography) the APX-CEXT refusal names a different package
per machine/run. Byte-exact product contract; T-14 assertions can go flaky.
Fix: collect all hits, sort by relative path, report lexicographic-first.
(Only nondeterministic emission found: zero HashMaps in the six crates;
lock + installed_versions are sorted.)

**F2. MED — ap-host rebuilds wasmtime Engine + JIT-compiles per call**
(ap-host/src/lib.rs:50-59). Fix: caller-owned `&Engine` / OnceLock;
precompile + cache (pairs with B4).

**F3. MED — `antipythony add` double-copies the staged tree despite the
same-volume comment** (main.rs:149-151 vs merge_dirs copy :255-271). Fix:
per-entry rename, copy only on EXDEV.

**F4. LOW — golden-dump String-per-node + Vec+join** (ap-ir/src/dump.rs:940
et seq.) — deterministic, constant-factor; writer-form refactor.
Folding digit-string round-trips (verify.rs:2640 etc.) LOW.

**Checked clean:** no AST/source retained inside ap_ir Module; native/wasm
codegen are stubs (watch F shapes when Cranelift lands); NESTED_DEDUPE,
iterator-registry prune, exhausted-gen retire, RE_CACHE/SRE_TABLE_CAP,
rss_guard, structmod, exc_hier.is_subclass, ast_bridge BRIDGE_SOURCE all
verified bounded/clean by auditors (parent spot-checked the key ones).

---

## Suggested wave plan (severity ÷ risk, respecting byte-exact gate)

- W1 quick per-op wins, low blast radius: A3 (env flags OnceLock),
  A5-partial (StopIteration fast path), D1-D5 (bytearray/list/str-subscript/
  singledispatch/dict-views in-place), D9 cheap subset (b16encode, len,
  receiver copies), F1 (sort — one line), E1-E4 (single drain-hook sweep),
  E5 (XPath self-carrier), C1 (O(1) addr lookup).
- W2 structural: A1/A2 (note_py_loc + Rc<FnEntry>), B1/B2/B3 (registry
  in-place, line_starts, verifier borrow walk), C2/C3/C4 (GC trace/roots/
  alloc — needs gc018-020 fences green).
- W3 cache + layout: B4 (IR compile cache — new subsystem, needs
  serialization format decision; gate: conformance bytes unchanged), C7
  (shrink), E6 (stack pool), A4 (interning — touches the most code; do
  after W1/W2 land so the corpus is a stable reference).
- W4 async/no-GIL pre-work: E7, C9, E5-cross-thread class bug — coordinate
  with leaf 02.09.

Each wave gated by: cargo test --workspace 0 failed; pytest full suite;
corpus non-decreasing; T1 scrapy file:// byte-compare; T2 default-config
verdict-identical. Baseline note above re the fence under load.
