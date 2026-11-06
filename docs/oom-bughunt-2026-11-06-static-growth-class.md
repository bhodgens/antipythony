# OOM Bughunt 2026-11-06 — Static growth-class audit of crates/ap-run/src

Static code-reading audit (no product code modified, no cargo runs). Goal: find
unbounded clone/accumulate patterns that could resurrect the GB-scale leak class
fixed in the 2026-11-05 OOM campaign (docs/import-leak-recheck-2026-11-05.md).

Method: full inventory of `thread_local!`/`static` containers in ap-run (354
candidate sites), then per-container proof of growth-vs-eviction by grepping the
whole `crates/ap-run/src` tree for remove/retain/pop/clear/drain sites. Deep
reads of interp.rs, frame.rs, generators.rs, iter.rs, builtins.rs,
builtins_modules.rs, classes.rs, methods.rs, importer.rs, threading.rs,
threading_call.rs, gcsupport.rs, sre.rs, lxml.rs, ast_bridge.rs. Top findings
were spot-verified first-hand against the source. Known-fixed classes (_ast
getattr clone, _ast self-import recursion, O(n^2) GC, importer post-insert)
excluded per scope.

**Headline: this is NOT a clean bill. Multiple GB-scale insert-only registries
remain, all sharing one shape: a thread-local table keyed by object identity
(address/counter) that holds `Value`/`Gc`/`Rc` strongly and is never evicted.
The ap-gc cycle collector cannot reclaim anything pinned by these tables, so
they are permanent roots.**

---

## GB-scale findings (ranked)

### G1. Iterator registry: `break`/exception pins a full container clone forever
- **Severity:** GB-scale. **Confidence:** high (verified first-hand).
- **Evidence:**
  - `frame.rs:288` — `pub iterators: RefCell<HashMap<u64, Gc<PyIterator>>>`.
  - `iter.rs:46-52` — materialization snapshot at registration:
    `Value::List(l) => IterState::Seq(l.items().to_vec(), 0)`,
    `Value::Tuple(t) => IterState::Seq(t.0.clone(), 0)` — the iterator **clones
    the entire container**.
  - `interp.rs:1026-1048` (`Stmt::For`) — `LoopOut::Break` sets `broke=true;
    break;` and `LoopOut::Escape` returns `StmtOut::Unwind` — **neither path
    removes the id from `interp.iterators`**.
  - Sole removal site: `interp.rs:8907` inside `pull_iter`, firing only when
    `it.next()` returns `None` (natural exhaustion).
- **Growth trigger:** every `for x in big_list: ... break` (or an exception
  thrown mid-loop, or an early `return`) leaks one full snapshot of the
  container, held by the interpreter until process exit. Scrapy pipelines and
  pytest loops break out of loops constantly. A crawl iterating a 100MB list and
  breaking per item pins 100MB per occurrence.
- **Fix sketch:** drop `its.remove(&id)` into the `LoopOut::Break`/`Escape`
  arms (and the `?`-unwind path) of `Stmt::For`/`Stmt::While`-comprehension
  loops; better, make `PyIterator` a plain `Gc` child of the frame so the GC
  reclaims it (removes the registry entirely).

### G2. sre MATCH_TABLE: every `re.Match` pins its full subject text forever
- **Severity:** GB-scale for regex-heavy workloads. **Confidence:** high.
- **Evidence:**
  - `sre.rs:2050` — `static MATCH_TABLE: RcRefCell<HashMap<usize,
    Rc<MatchBox>>>`; `MatchBox` carries `pub text: String` — the **whole
    searched string** — plus pattern Rc and result (`sre.rs:2053-2058`).
  - Insert: `sre.rs:2237` (`match_object`), keyed by object address. **Zero
    removal sites** in the file (grep: only `pop` sites are unrelated parser
    stack ops).
  - Same file: `PAT_TABLE` (`sre.rs:2049`, inserts at :2040/:2701) accumulates
    every `re.compile`d pattern forever; the declared `RE_CACHE` (`sre.rs:2017`)
    is **dead code** — zero reads — so there is no CPython-style 512-entry
    pattern dedup at all.
- **Growth trigger:** `re.search/match` per line/cell over large corpora
  (log processing, scrapy response parsing, pytest collection). Each match
  retains its full subject text; one match per KB-sized line over 1M lines
  pins ~1GB.
- **Fix sketch:** stop tabling plain matches (resolve `group()`/`span()` via a
  Weak keyed handle or store the text as `Rc<str>` shared with the caller);
  evict `PAT_TABLE` via weak handles; wire `RE_CACHE` with the CPython 512 cap
  or delete it.

### G3. lxml DOCS registry: every parsed document's full DOM pinned for process life
- **Severity:** GB-scale for crawls. **Confidence:** high.
- **Evidence:**
  - `lxml.rs:1505` — `static DOCS: RefCell<DocRegistry>` where
    `DocRegistry { docs: Vec<SharedDoc> }`, `SharedDoc = Rc<RefCell<LxDoc>>`
    (the whole arena-backed DOM).
  - `lxml.rs:1509-1515` — `register_doc` **push-only**; grep confirms no
    removal of `docs` entries anywhere in the file (the `.pop()/.clear()` hits
    are parser/document-content stack ops, not registry eviction).
- **Growth trigger:** the scrapy T1 crawl parses one `TextResponse` per fetched
  page. Every page's DOM stays resident for the whole crawl — exactly the
  workload class the OOM campaign fenced. 10k pages × 100KB DOM = ~1GB.
- **Fix sketch:** key elements by `Weak<RefCell<LxDoc>>` (upgrade in
  `elem_parts`) so docs die with their element graph, or refcount doc handles
  and pop on last element drop.

### G4. Per-execution growth of `nested_functions` + `defining_frames` + `lambda_bodies`
- **Severity:** GB-scale. **Confidence:** high.
- **Evidence:**
  - `frame.rs:435-441` — `register_nested`: `id = NESTED_BASE + t.len();
    t.push(entry)` on `nested_functions: RefCell<Vec<FnEntry>>`; `FnEntry.def`
    is a full `FnDef` AST clone (`(**f).clone()`, `interp.rs:1447`).
  - Insert sites: `interp.rs:1446` (every execution of `Stmt::LocalDef`),
    `interp.rs:3230` (every evaluation of `Expr::Lambda`). Companion tables
    `lambda_bodies: HashMap<u32, Expr>` (`frame.rs:313`) and
    `defining_frames: HashMap<u32, Rc<Frame>>` (`frame.rs:315`) are filled at
    the same sites — the frame Rc pins the whole locals map. **No
    remove/clear for any of the three anywhere in the crate.**
- **Growth trigger:** any `def`/`lambda` evaluated inside a function called per
  item — scrapy callbacks defining helpers, pytest fixture factories,
  `sorted(x, key=lambda ...)` inside a hot function. Every call appends a fresh
  AST clone + frame pin. 1M calls × ~1KB = ~1GB, with live-Value pins on top.
- **Fix sketch:** intern nested defs per (function-instance, def) pair — cache
  the id on the closure/function object instead of appending per execution;
  keep `defining_frames` as `Weak<Frame>`.

### G5. CELL_BACKING: insert per `__closure__` attribute access, never evicted
- **Severity:** GB-scale in introspection-heavy loops. **Confidence:** high.
- **Evidence:**
  - `interp.rs:23` — `pub static CELL_BACKING: RefCell<HashMap<usize,
    CellSlot>>` (thread-local).
  - `interp.rs:11755-11762` — every evaluation of `f.__closure__` creates a
    **fresh** cell PyObject and inserts `Rc::as_ptr(&c) as usize → cell.clone()`
    (a strong `CellSlot` pinning the captured `Value`, outside the GC heap's
    reach for that pin).
  - No removal site in the crate (reads at `interp.rs:12266`,
    `classes.rs:3465`).
- **Growth trigger:** `inspect`-driven frameworks read `__closure__` per
  function per operation (pytest/pluggy hook inspection, functools). Same
  leak shape as the fixed `_ast` getattr clone, but keyed per access — each
  access mints a new key, so even a single function in a loop grows the map
  linearly.
- **Fix sketch:** cache the cell-wrapper object on the function object itself
  (stable per function), or store `Weak` in the registry and evict dead keys
  on insert.

### G6. `interp.stdout` buffers the entire program's output until exit
- **Severity:** GB-scale for chatty long-running programs. **Confidence:** high
  (verified first-hand; **design intent**, so confirm before changing).
- **Evidence:** `frame.rs:295` `pub stdout: RefCell<Vec<u8>>`;
  `frame.rs:414` `write_out` extend-only; the buffer is flushed exactly once at
  `interp.rs:139` (`stdout.write_all(&interp.stdout.borrow())`). No mid-run
  clear/drain anywhere.
- **Growth trigger:** any long-running program that prints per item (progress
  logging in a crawl, verbose pytest). 1GB of print output = 1GB RSS before the
  single end-of-run flush.
- **Fix sketch:** flush+clear the buffer to the real stdout on a byte threshold
  (e.g. 1MB) or on newline when stdout is not a captured capture-stream; the
  S-06-2 byte gate only needs capture-streams buffered, not the tty path.

### G7. WORKER_REG_IN forwarding hook: unbounded channel between two drain sites
- **Severity:** GB-scale potential. **Confidence:** high on mechanism, medium
  on real magnitude.
- **Evidence:** `gcsupport.rs:1680-1721` — after the first
  `install_worker_thread`, `gc_register_forward_worker` becomes the
  process-global allocation hook: **every** new `Value` on any thread is
  `tx.send`-cloned into the channel. `drain_worker_registrations`
  (`gcsupport.rs:1699`) is called at exactly two sites:
  `threading_call.rs:189` and `threading_call.rs:1581`.
- **Growth trigger:** a workload that spawns one thread, then does
  import/alloc-heavy work with no further threading/queue dispatch (e.g. a
  worker that imports scrapy and parses). Channel grows without bound;
  contents are GC-immune (owned by the channel).
- **Fix sketch:** also call `drain_worker_registrations()` from the ap-gc
  collect entry and/or when channel length crosses a threshold; stop
  forwarding when no worker entries are live.

### G8. `generators` map: exhausted/abandoned generators never retire
- **Severity:** GB-scale for generator-heavy workloads. **Confidence:** high
  (two independent audits converged).
- **Evidence:** `frame.rs:289` `generators: RefCell<HashMap<u64, GenFrame>>`;
  inserts `interp.rs:3277/7136/7793`; `generators.rs:465-466` — `resume_live`
  **re-inserts unconditionally**, keeping `GenFrame::Live { fiber: None, .. }`
  entries when `fiber.done()`. `GenFrame`/`GenState` pin `func_obj`, `args`,
  `kwargs`, `super_ctx`, and the defining `Rc<Frame>`. Removal sites
  (`interp.rs:7881/7950/8069`) are materialize/ensure paths that re-insert.
- **Growth trigger:** scrapy yields thousands of short-lived generators per
  crawl; genexprs created but never iterated keep their full capture forever.
- **Fix sketch:** when `fiber.done()`, drop the table entry (retain only the
  `si_value` the StopIteration protocol needs, on the iterator object itself).

### G9. `lru_cache(maxsize=0)` caches unboundedly (CPython caches nothing)
- **Severity:** GB-scale pathological / MB-scale typical. **Confidence:** high.
- **Evidence:** `builtins_modules.rs:12898-12913` — always
  `s.cache.insert(key, val.clone()); s.order.push(key)`; eviction runs only
  `if maxsize.map(|m| m > 0).unwrap_or(false)` — so `maxsize=Some(0)` inserts
  forever, never evicts (`Some(0)` passes through the two-stage parse at
  :4860-4882 unchanged). CPython: `maxsize=0` means "no caching".
- **Growth trigger:** vendored stdlib code that passes `_MAXCACHE`-style or
  explicit `maxsize=0`, called with distinct args in a loop — every result
  Value and its object graph pinned in `LRU_STATE`.
- **Fix sketch:** treat `Some(0)` as don't-insert; change the gate to
  `if matches!(maxsize, Some(_))`.

### G10. `tb_frames` head-insert leak per generator-raised exception (+ O(n²))
- **Severity:** MB-to-GB; quadratic CPU on top. **Confidence:** medium-high
  (mechanism verified first-hand: `generators.rs:308-311` does
  `interp.tb_frames.borrow_mut().insert(0, ...)`; cleanup sites
  `interp.rs:1110/1698` (`truncate(tb_snapshot)`) and `interp.rs:1725`
  (`drain(tb_snapshot..end)`) operate on indices **above** the pre-resume
  snapshot, while the generator's entry lands at index **0, below it**).
- **Growth trigger:** every caught exception that traversed a generator frame —
  pluggy hooks are generators, i.e. the entire pytest hook system. Each event
  leaks one `(String, usize, String)` entry and shifts the whole Vec (O(n)
  per insert).
- **Fix sketch:** record `pre_len = tb_frames.len()` before the generator
  resume and truncate to `pre_len` after (not the pre-thread snapshot), or
  append+reverse at traceback materialization instead of `insert(0)`.

## MB-scale findings

| # | Site | Expression | Trigger | Confidence | Fix sketch |
|---|------|-----------|---------|------------|------------|
| M1 | `threading.rs:185` LOCALS | `local_set` (:978-987) inserts per (local-addr, tid); no eviction on object death or thread exit | per-request `threading.local` (web frameworks, scrapy middlewares) | high | evict per-tid maps in `worker_bootstrap` teardown; sweep outer keys by GC liveness at major GC. Addr-recycling can hand a dead local's data to a new object (correctness) |
| M2 | `threading.rs:178` THREADS | push at :236/:533-547; only `retain` is spawn-failure rollback (:576-579); `entry_for_current` (:768) O(n) scan per threading call | thread-pool churn | high | retain `!done` entries in `wait_for_non_daemons` |
| M3 | `threading.rs:791` OBJ_MIRROR | `mirror_put` at :1088 per Thread/Timer ctor; no removal; pins thread object + `<target>`/`<args>`/`<kwargs>` attrs | thread churn with large args | high | evict `done` entries at drain/major-GC turn; `mirror_get` by recycled addr can return an unrelated newer object (correctness) |
| M4 | `threading.rs:348` PRIMS | insert at :353/:366 + threading_call.rs:321-1602 (10 sites); zero removals; `QueuePrim.items: VecDeque<Value>` holds Values strongly | per-request locks/queues | high | evict keys absent from GC registry at major GC |
| M5 | `builtins_modules.rs:503` NAMED_CALLABLES | fallback `<ap-named-{counter}>` path (:542-548) mints a fresh immortal entry per call for bound methods/partials; inserts also :3302/:3308; zero removals | `functools.partial`/`encode_arg` (:17736)/itertools over bound methods in a loop | high | key by object address so repeat registrations dedup |
| M6 | `builtins_modules.rs:9994` LINECACHE | whole file cached as `Vec<String>` per path; `checkcache` is a no-op (:10041); `cache_lines` returns `v.clone()` (:10058) — deep copy of every line on **each** `getline` | traceback-heavy runs over many files | high | return `Rc<Vec<String>>`; implement mtime checkcache; cap total bytes |
| M7 | `gcsupport.rs:51/53` WEAK_TOKENS / FINALIZERS | push-only (`weakref.ref/proxy` :163; `finalize` :837+; `detach`/call only set flags :860-905); no retain/prune of dead or fired entries | ABC/WeakSet-heavy scrapy+pytest chains | high | `retain` dead/fired entries in `fire_dead_weakrefs` and after atexit drain |
| M8 | `builtins.rs:2798` INTERNED | `sys.intern(x)` inserts `String → Value` forever and accepts **non-str** values unvalidated (pins an arbitrary object under its stringified key; CPython raises TypeError) | parsers/ORMs interning many unique strings | high | type-gate to `Value::Str` before insert |
| M9 | `builtins.rs:2191/2222/5198` iterator registration via `next()` | `next_registered` (:1618-1626) pulls via `it.next()` and does **not** remove on exhaustion (removal exists only in `pull_iter`, `interp.rs:8909`); `register_iter` materializes the whole iterable first (`IterState::Seq` snapshot) | any `iter(x)`/`next()` protocol use not driven via for-loop; itertools chains | high | mirror `pull_iter`'s remove-on-exhaustion in `next_registered` |
| M10 | `interp.rs:192` FILE_SRC | whole source cached per path, no eviction | bounded by distinct files (tens of MB over the scrapy import chain); grows with reload/generated-file churn | high | LRU-cap or Weak-key by module |
| M11 | `lxml.rs:2891` XPATH_OBJECTS | push per `etree.XPath(query)` ctor, never removed | scrapy selectors compile XPaths per parse | high | dedup by query string |
| M12 | `interp.rs:7656` PENDING_GEN_ERROR | staged `ExceptionValue` (args = Values) for a generator never resumed again is drained one-at-a-time at :7702 | abandoned errored generators | medium | evict staged entries when the fiber is dropped |

## Noise (documented for completeness)

- `builtins_modules.rs:11003` POLL_TABLES — insert-only (:11137) but bounded by
  poll-object count; ctor overwrites on addr reuse. `select.poll` has no
  `close()` by design (:11148).
- `interp.rs:13836` MODULE_FRAME_CELLS — keyed by (frame ptr, cell name);
  grows only if modules are evicted and re-imported (importlib.reload churn).
- `builtins_modules.rs:6435` LAST_JSON_ERR — single slot; pins one full parsed
  doc (MBs for large responses) until next error.
- `threading.rs:195` CANCELLED — usize set never cleared; **one-line fix**:
  stale flag can make a recycled-addr Timer's `wait_cancellable` return
  instantly (:666) — correctness nit.
- `threading.rs:81` TIDMAP — tiny per-ThreadId entries, never removed.
- `gcsupport.rs:1041` INTERNED_TYPE_PINS — duplicate pushes per weakref-to-Type;
  types are immortal anyway.
- `builtins_modules.rs:6433` EXC_ATTRS + `json_error` — dead code (only writer
  never called; no readers). Remove.

## Correctness bugs found adjacent to the leak surfaces (out of scope, filed here for the parent)

1. `classes.rs:1406` clears `CLASS_KWARGS` **before** :1417 clones it for the
   metaclass `__init__` call → metaclass `__init__` always receives empty
   kwargs (CPython passes full kwds). Early `return Err` at :1359-1362 and the
   `?` at :1117 strand the previous class's kwargs (bounded, replace-on-next-def).
2. `methods.rs:1001-1013` `super_stack`: one push at :1004, **two** pops at
   :1012-1013 — the second pop removes a foreign entry (caller's or sentinel),
   unbalancing shared `super()` resolution state.
3. `sre.rs:2017` RE_CACHE declared but never read (dead) — no pattern dedup
   exists at all (also listed as G2 evidence).

## Verified clean / bounded

`generators.rs` ACTIVE/PARKED/CURRENT stacks (push/pop symmetric);
`scope_cache` (`interp.rs:279`, per-registry lifetime); `Box::leak` scope
tables (`interp.rs:474`, one per module exec — bounded by module count, but
note re-import churn multiplies it); `type_objects`/`exc_classes`/
`user_class_values`/`user_subclasses` (keyed by name, overwrite on redefinition;
`user_subclasses` (`frame.rs:335`, push per class at `classes.rs:1452`) grows
only under dynamic class creation — `type()`/namedtuple in a loop would grow it,
flag for T3/T4); `CURRENT_CLASS_NS` (cleared before error propagate,
`classes.rs:895-897`); `MODULE_CACHE`/`SYS_MODULES` (module-registry-shaped;
failed-body removal present at `importer.rs:691/:1292`); `ATEXIT_FNS` (drained
LIFO at exit, `interp.rs:570`); `WORKER_INTERP`/`EXCEPTHOOK`/`SYS_PATH_VIEW`/
`BRIDGE_SOURCE`/`JSON_SEPS`/`JSON_DEFAULT_HOOK`/`MT`/`URAND_STATE` (single-slot
or scalar); `JOIN_CV/JOIN_MU`; `threading_call.rs:306` START OnceLock;
worker-thread TLS freed on thread exit (`threading.rs:604`).

## Recommended fix order

1. G1 (break-path iterator leak) + M9 — same registry, cheap, huge win.
2. G3 (lxml DOCS) — directly protects the T1 scrapy crawl gate.
3. G8 + G2 — generator retirement + sre match-text pinning (biggest steady-state
   RSS for pytest/scrapy).
4. G4/G5 — per-execution AST/closure accumulation.
5. G7/G6 — drain hook + stdout flush policy (G6 needs a design check against
   the S-06-2 byte gate).
6. G9/G10 and the MB table, then the two correctness bugs.

---

# WAVE 1 FIX (2026-11-06, same day)

Scope: G1 (+F5/M9 pull-path family) and G3. Probes run via
`plans/robustness-ci/measure_rss.py`, env caps
`AP_MAX_RSS_GB=2 AP_PROBE_MAX_RSS_GB=2`, serial. BEFORE = tree @ 4709a20,
AFTER = wave-1 commits below. Raw logs /tmp/oomwave1/results_*.txt.

## WAVE 1 FIX — G1 + F5/M9 (iterator registry)

Commit: 090ad43 (fix(run): G1+F5 iterator registry).
Approach: (a) Stmt::For Break/Escape arms now drop the registered id
(generator ids excluded — their lifecycle is the protocol's); (b)
PyIterator gains `table_id` + a Drop hook recording dead ids;
`prune_dropped_iters` (pull_iter) and `next_registered` drain them, so
abandoned iter()/next() chains no longer pin state; (c) iter.rs stops
cloning containers: `IterState::SharedList`/`SharedTuple` hold the
container's own Rc and clone only the yielded Value per step.

Before/after (p_break_list: for+break over a 1000-element list, N loops):

| N     | before maxrss_kb | after maxrss_kb |
|-------|------------------|-----------------|
| 5,000 | 93,421           | 10,207          |
| 20,000| 342,409          | 10,158          |
| 50,000| 839,614          | 10,125          |

(+16.6KB/iteration growing -> FLAT at interpreter baseline.)
Tests: t05 192P/1x/6x, t06 194P/50x, t04 44P/3s, t08 154P/323x, t07
131P/23x, t11 284P — zero regressions (the Shared borrow-per-step keeps
PyList RefCell semantics; mutation-during-iteration caveat unchanged).

## WAVE 1 FIX — G3 (lxml DOCS registry)

Commit: eb6f7b4 (fix(run): G3 lxml DOCS registry).
Approach: registry slots hold `Weak<RefCell<LxDoc>>`; strong refs live in
a bounded LRU pin window (`DOC_PIN_WINDOW` = 64 docs, oldest evicted when
a new parse lands). A crawl that parses-uses-drops pages keeps ~1 DOM
resident instead of every DOM it ever parsed; table-heavy scripts (<=64
live documents) are unaffected. Freed slots are reused so the slot Vec
itself stays bounded. Stale doc_ids (pin evicted + no live handles)
resolve None cleanly (attribute-miss), never alias a newer document.

Before/after (p_lxml_docs: 500-element doc parsed and dropped per loop):

| N      | before maxrss_kb | before note    | after maxrss_kb |
|--------|------------------|----------------|-----------------|
| 2,000  | 333,529          | —              | 23,543          |
| 10,000 | 1,621,442        | —              | 30,081          |
| 30,000 | 2,167,111        | RSS-GUARD ABORT| 38,027 (rc 0)   |

(~45x/54x reduction; the 30k case went from 2GB-cap kill to flat.)
Tests: t11 stdlib lxml arms green (284 passed).

## WAVE 1 co-fixes landed by the parallel wave-2 agent (same bughunt docs)

- sre.rs G2 (PAT_TABLE/MATCH_TABLE): wave-2 initially stored `Weak` with
  nothing holding the strong Rc — re.Pattern/re.Match surfaces broke
  immediately ("bad receiver for Pattern.match" on any pattern-object
  call; verified against a clean HEAD worktree). Wave-1 corrected the
  storage back to strong Rc + a 1024-entry FIFO eviction ledger
  (uncommitted — left in the wave-2 agent's working set with their
  sre.rs commit).
- builtins_modules.rs M5/G9/M6 + the G2 dispatch arm: landed inside the
  wave-1 F3 commit 72b6fb5 (shared file; co-landing documented there).

---

# WAVE 2 FIX (2026-11-06, same day)

Scope: the remaining growth classes — G2 final state, G4, G5, G6, G7, G8,
G9 verification, G10, F4. Probes via `plans/robustness-ci/measure_rss.py`,
env caps `AP_MAX_RSS_GB=2 AP_PROBE_MAX_RSS_GB=2`, strictly serial (cargo
preflight `pgrep -fl 'cargo|rustc'`), no background processes, single test
kill at 300s. BEFORE = wave-1 commits (f7ea3a3), AFTER = wave-2 commits
below. Probe sources /tmp/oomwave2/.

## WAVE 2 FIX — G2 final (sre MATCH/PAT tables + RE_CACHE)

Commit: eece027 (fix(run): G2 sre registries).
Approach: MATCH_TABLE/PAT_TABLE keep STRONG Rc (required — the PyObject has
no carrier for the payload; the wave-1-corrected design) but are bounded by
SRE_TABLE_CAP (1024, oldest-first ledger eviction; recycled addresses
overwrite). RE_CACHE wired as the CPython-style bounded (512-entry)
(pattern, flags) pattern cache consulted by module-level re.compile —
`re.compile(p) is re.compile(p)` now holds (CPython-identical, verified).

Before/after (g2_flat: search-and-drop, ~130B subjects):

| N      | before maxrss_kb | after maxrss_kb |
|--------|------------------|-----------------|
| 20,000 | 33,292           | 11,501          |
| 100,000| 117,211          | 11,517          |
| 200,000| 217,219          | 11,681          |

(1.1KB/match linear -> FLAT at baseline.)
Tests: re_differential 42/42, re_fuzz 4/4, method_surface_differential
34/34, stdlib slice s010/s011 PASS byte-identical.

## WAVE 2 FIX — G4 (nested def/lambda per-execution accumulation)

Commit: c80550c (fix(run): G4 nested def/lambda dedupe).
Approach: NESTED_DEDUPE thread-local keyed (AST-node address,
defining-frame address) — CPython likewise mints a new function OBJECT per
execution but shares one CODE object; the (node, frame) pair is our
code-object identity. register_nested_for (Stmt::LocalDef) and the inlined
dedupe (Expr::Lambda) reuse the entry on re-execution.

Before/after (g8c_ctl: 200k inner-def executions, no generators):

| N       | before maxrss_kb | after maxrss_kb |
|---------|------------------|-----------------|
| 20,000  | 36,732           | 11,321          |
| 100,000 | 132,202          | 11,190          |
| 200,000 | 243,646          | 11,157          |

(1.1KB/call linear -> FLAT.)
Tests: classes_differential 29/29, name_resolution_differential 24/24,
closure semantics verified vs CPython.

## WAVE 2 FIX — G8 (exhausted generators never retire) + F5 Drop-hook

Commit: c96f41c (fix(run): G8 exhausted generators retire).
Approach: resume_live drops the interp.generators entry when fiber.done()
(the StopIteration value lives in interp.si_value, not the GenFrame);
the resume-not-live arm answers CPython finished-generator semantics
(send/next -> StopIteration, throw(e) -> re-raise e per G001's
"exhausted-throw late" pin, close() -> Ok). Also fixed wave-1's F5 Drop
hook: PyIterator::drop used DEAD_ITER_IDS.with, which aborts (AccessError,
rc=133) for iterators dropped during interpreter teardown — try_with now.
NOTE: wave-1's F1 stack-size fix bounds the RESERVATION; wave-2's G8
bounds the REGISTRY LIFETIME — the churn probe below is the combined win.

Before/after (g8_churn: 200k create+exhaust def-generators):

| N       | before maxrss_kb | after maxrss_kb |
|---------|------------------|-----------------|
| 20,000  | 47,562           | 11,419          |
| 100,000 | 187,645          | 11,452          |
| 200,000 | 358,662          | 11,550          |

(1.7KB/gen linear -> FLAT.)
Tests: G001-pep342-generators byte-identical vs CPython 3.14 (full
protocol), classes_differential 29/29, ap-gc 23/23.

## WAVE 2 FIX — G5 (CELL_BACKING per-access growth)

Commit: 67d3e09 (fix(run): G5 cell wrapper cache + G6 stdout).
Approach: CELL_WRAPPER_CACHE keyed (closure_slot, cell name) — the doc's
"cache the wrapper on the function object" arm; dead wrappers swept by GC
registry membership on insert. Wrapper identity per access is now stable;
verified CPython 3.14 also answers `f.__closure__[0] is f.__closure__[0]`
True, so this is byte-identical, not a divergence.

Before/after (g5 probe: 200k __closure__ reads over 100 live closures):
FLAT 11.2-11.5MB across 20k/100k/200k (the pre-fix table grew one
PyObject + registry entry per ACCESS — linear in reads).
Tests: closure probe vs CPython byte-identical; classes suites green.

## WAVE 2 FIX — G6 (stdout buffers the whole run)

Commit: 67d3e09 (same).
Approach: write_out drains interp.stdout to fd 1 past a 1MB threshold;
drained bytes are kept in stdout_flushed_prefix and re-prepended into
RunOutcome.stdout at the end-of-run flush — outcome bytes byte-identical
to the unbuffered behavior, S-06-2 byte gate intact. Both production
callers (ap-cli, ap_run_probe) pass real stdout; differential suites
consume the subprocess's fd 1, so ordering is preserved.

Before/after (g6: 120k print lines): the BUFFER is bounded at ~1MB by
construction (was: whole-run buffer, 1GB of print = 1GB RSS); output line
count and content byte-identical vs CPython.
Tests: import_differential 6/6, stdlib s005 PASS.

## WAVE 2 FIX — G7 (WORKER_REG_IN drain sites)

Commit: 86c4cdc (fix(run): G7+G10+F4).
Approach: drain_worker_registrations() now also runs on every gc.collect()
dispatch and at the run tail (was: only 2 threading_call sites — a
spawn-once thread doing import/alloc-heavy work grew the channel
unbounded; contents GC-immune).
Tests: gc_invariants 9/9 (worker wiring); threading not probed (sibling
WIP territory, per the RSS doc's "not covered" note).

## WAVE 2 FIX — G10 (tb_frames index-0 insert leak + O(n^2))

Commit: 86c4cdc (same).
Approach: resume_live records the tb_frames depth at resume entry
(GEN_TB_BASE); the generator-raise arm inserts its frame THERE instead of
index 0 — the frame now sits inside the driver's cleanup window, so
try/except truncate removes it (was: one leaked entry + O(n) shift per
caught generator exception; pluggy hooks are generators = the entire
pytest hook system, O(n^2) cumulative).

Before/after (t_g10: 200k caught gen-raised exceptions): FLAT 11.5-11.6MB
across 20k/100k/200k (linear growth before).
Tests: G001-pep342-generators byte-identical (throwee/exhausted-throw
traceback shape unchanged).

## WAVE 2 FIX — F4 (setrecursionlimit native-stack backstop)

Commit: 86c4cdc (same).
Approach: enter_call derives a hard depth cap from the interpreter-thread
stack budget (256MB reservation; 256KB/frame debug, 50KB/frame release) —
setrecursionlimit still lowers freely, but a raised limit can no longer
push past the native stack.

Before/after (p7 probe from the RSS doc — setrecursionlimit(5100) +
f(5000)):

| depth | before                    | after                   |
|-------|---------------------------|-------------------------|
| 5,000 | SIGABRT rc=-6             | clean RecursionError    |
| 1,020 | ok                        | ok                      |
| 1,300 | SIGABRT                   | clean RecursionError    |

Default-limit behavior unchanged (RE at 1000, byte-identical vs CPython).
Tests: t_f4 vs CPython; classes/re suites green.

## WAVE 2 skipped / noted

- G9 (lru maxsize=0): landed in wave-1's 72b6fb5 (shared file) — verified
  post-commit: g9b probe byte-identical vs CPython (maxsize=None caches,
  maxsize=0 caches nothing, currsize 0).
- M8 (sys.intern non-str): landed in wave-1's 090ad43 — verified: TypeError
  "intern() argument must be str" byte-identical vs CPython; m8_intern
  probe FLAT 10.4MB @200k.
- M1-M4 (threading LOCALS/THREADS/OBJ_MIRROR/PRIMS): NOT touched —
  threading.rs/threading_call.rs are sibling WIP per AGENTS.md, same reason
  the RSS doc skipped probing them.
- M6 (LINECACHE): landed in 72b6fb5; wave-2 verified the checkcache
  semantics byte-identical vs CPython (append+checkcache+getline chain).
- M11 (lxml XPATH_OBJECTS): out of wave-2 scope (lxml.rs was wave-1's G3
  file; XPATH dedup not filed as GB-scale — left for a follow-up).
- M7 (WEAK_TOKENS/FINALIZERS prune): covered by wave-1 F2 (7eb65a4).
- Correctness bugs 1/2 (classes.rs super_stack, metaclass kwargs): FORBIDDEN
  FILES (sibling stash@{0} WIP) — untouched per instructions.
- Pre-existing non-leak failures observed in the stdlib slice (s013
  docstring-on-Function + cmp_to_key order, s014 namedtuple repr via
  vendored functools): confirmed NOT wave-2 regressions — zero diff hunks
  touch those paths; namedtuple repr verified broken independent of wave 2
  (plain namedtuple probe). Filed here for the parent's triage.

