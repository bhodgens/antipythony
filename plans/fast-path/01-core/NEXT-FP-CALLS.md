# NEXT-FP-CALLS — leaf session record (2026-11-26)

Leaf: plans/fast-path/01-core/02-calls-and-attrs.md
Branch: tier1-fp-calls (worktree /Users/caimlas/.git/worktrees/leaf-fp-calls,
CARGO_TARGET_DIR=target-fp-calls), base 4570bd4.

## What landed

### 1. Lowerer (crates/ap-bytecode/src/lower.rs)

- Param-kind refusals LIFTED: defaults / VarArg / KwArg / KwOnly params
  lower fine — the call boundary's binder (the tree-walker's exact
  machinery) binds every param name into the frame's locals map (defaults
  via eval_default, vararg as a tuple, kwarg as a dict) and run_frame's
  prefill reads slots BY NAME (visible_slot_of) instead of positional
  order only.
- StoreAttr (34th opcode): `obj.attr = v` lowers; value under obj; routed
  to classes::attr_set_protocol (the walker's exact store arm). Attribute
  AUG-targets lower LoadAttr (were refused).
- CallFunctionKw (35th opcode): kwargs'd calls marshal a FRESH CONTIGUOUS
  name-table slice [kw_first, kw_first+n_kw) + the kwarg values after the
  positionals. The slice is always freshly reserved (intern_name dedup
  could return an earlier index for an already-interned name and desync
  the pairing — regression-pinned).
- Set/Dict displays lower (BuildSet/BuildDict) — the last display
  refusals; `d = {}` in a loop body was silently tree-walking whole fns.
- Comprehension lowering: ListComp/SetComp/DictComp -> INLINE loop shape,
  NO separate function frame. Implicit scope emulated with comp_scopes
  (name -> hidden slot shadow map) + hidden slots past visible n_locals
  (excluded from local_names, so sync_slots_into_frame never leaks the
  comp's targets into the enclosing frame — S-02-2). Outermost clause's
  iterable evaluates in the ENCLOSING scope; inner iterables/filters/
  element inside the comp scope; inner-clause exhaustion resumes the
  outer loop (its handle stays on the stack); filters JumpIfFalse to
  their clause's ForNext. List/set accumulate via the FUSED LoadAttrCall
  append/add (receiver pushed BEFORE elt); dict via hidden temp slots +
  StoreSubscript (value/acc/key order). Non-name comp targets and
  walrus-in-comp stay loud fallbacks; GeneratorExp stays tree-walked
  (fiber machinery, master.md).

### 2. Execution (crates/ap-run/src/bytecode_interp.rs)

- StoreAttr arm: pops obj then value -> attr_set_protocol.
- CallFunctionKw arm: pops n_kw VALUES, REVERSES, then zips with
  names[kw_first..] — zipping during pops handed the LAST value to the
  FIRST name (caught: vendored _py_warnings import crashed under the
  toggle because category= got append's value).
- LoadAttrCall fast route EXTENDED (leaf core perf): primitive receivers
  (str/bytes/int/bool/float/tuple/list/dict/set/frozenset) dispatch
  methods::call_method (upper/lower/join/format/partition/encode/
  translate/__len__ direct arms) or call_builtin_values (receiver as
  args[0]) DIRECTLY — skipping the attr walk + fresh BoundMethod mint
  (~1.5us/call). Gate = builtin_value_method_exists_pub (NEW pub seam on
  the SAME (tag,name) table the mint path consults — one table, two
  callers, no duplicated family lists). Any shape the table doesn't
  claim falls to the exact mint route (identical semantics).
- fast_bound_mint (LoadAttr bare reads) extended with the same families;
  value-table families mint DOTTED marker names (pystr.startswith —
  native_attr's exact mint; bare names for the list/dict/set families
  call_method answers natively).
- FIX (pre-existing, exposed by the leaf's unbound probe): cur_line now
  follows dense_lines PER INSTRUCTION (one Vec index). Before, cur_line
  was initialized at frame start and NEVER advanced — every error exit
  and instrument sync reported the def-line region.

### 3. Gate (tests/bench/primitive/fix_bytecode.py)

3 new probes (12 total): methodcalls (dict/str/bytes/int method
families, hoisted bound methods, nested calls, attr+subscript chains,
defaults/kwargs/vararg callees), comps (list/set/dict shapes, filters,
nested clauses, scope non-leak, comp-of-comp, method-call bodies),
attrstore (StoreAttr/LoadAttr roundtrips, aug-through-attr, subscript
store chains). Three-way (tw/bc/CPython) byte-identical, 0 mismatches.

## Perf (release, best-of-5 medians, quiet window)

- dict.get loop (200k, d={}+set+get): tw 0.697s -> bc 0.044s = 15.89x
- dict set+get loop (200k): 11.88x
- str method-call loop (100k, upper/find/count/split/join/replace/index):
  tw 2.184s -> bc 0.245s = 8.93x
- str.split+join loop: 5.05x; str.startswith/find loops: 15-17x
- manual append loop (500k): 16.8x (fp-controlflow baseline, unchanged)
- listcomp 100k: 1.45x (tw's comp path was already fast; the WIN shows
  vs the manual loop: comp 0.099s == manual-append 0.099s in bc)
- Targets MET: dict rows >= 2x, str method loops >= 2x.

## Gates (all measured this session, release build)

- fix_bytecode 12 probes: three-way identical, 0 mismatches (incl. the
  APX_TIER_STATS exercise assert).
- fix_int / fix_json (127/127) / fix_sort (42/42) / fix_random /
  fix_bz2lzma / fix_crc (58/58) / fix_bytes (byte-identical to pre-leaf
  reference) / fix_sha_zlib (byte-identical pre/post) / diff_b64 (2132/0)
  / diff_hex (1976/0): ALL GREEN.
- ap-conform: values 50/50 + stdlib 48/48, BOTH toggles.
- cargo test ap-bytecode 52/52, ap-run 18 suites 0 failed (265+ passed).

## Honest gaps / filed follow-ups

1. Nested-loop ForNext handle-cache thrash (PRE-EXISTING fp-controlflow,
   exposed by this leaf's benching, NOT touched here): `for row in m:
   for v in row` at 100k rows is 2.3x SLOWER in bc than tw (0.425s vs
   0.183s) — the single-slot cached_it invalidates every alternation, so
   each step pays the iterator-table Mutex; the exhaustion prune adds a
   lock per inner loop. Owned by 02-perf (multi-slot handle cache /
   inline SharedList cursor at GetIter when the iterable is a plain
   local list).
2. User-instance attr loops (b.v = i; t += b.v): 1.0x — attr_set_protocol
   / attr_get walk IS the semantics (setattr/descriptor/MRO); a guarded
   instance-dict fast arm is possible but riskier; filed for 02-perf
   inline caches.
3. kwargs-call loops (f(i, c=1)): 0.98x — dominated by the call-boundary
   binder (shared machinery), identical route in both walkers. A fast
   binder path = call_value surgery, out of leaf scope.
4. Comprehension loop bodies show 1.1-1.6x vs tw's dedicated comp path
   (the tw comp was never slow; the win is vs the MANUAL loop they
   replace, 16.8x). Nested-clause comps pay gap #1 on the inner GetIter.
5. IfExp / walrus-in-comp / non-name comp targets / genexpr /
   *args-**kwargs call sites: still loud whole-function fallbacks
   (unchanged scope boundaries).

## Commits (branch tier1-fp-calls)

- 8f57ef6 fp-calls 1: Call/LoadAttr/StoreAttr/Subscript opcodes +
  comprehension lowering
- c9f8738 fp-calls 2: set/dict displays + kw-slice pairing fix + cur_line
  fix
- <this> fp-calls 3: regression tests — empty dict/set displays, kw-slice
  contiguity
- <this> gitignore: fix_crc's driver-generated ppcrc_data.bin
