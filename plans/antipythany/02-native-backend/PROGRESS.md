# Plan 02 progress log

## Pon study - COMPLETE (committed 1237c75)
- docs/pon-study-lowering.md (185 L) + docs/pon-study-tier1.md (208 L), clone
  d0a17177, all cited paths verified, zero verbatim copies.
- KEY ADOPTABLES (lowering): generator state-machine transform, finally
  trampoline, __classcell__, match eager-partial-bind. Parts-not-elements
  gotcha: investigated, NOT a bug in us.
- ARCHITECTURAL FORK: pon flat-CFG vs our structured ap-ir. 02.05 implementer
  decides; default = structured, documented.
- TIER-1 TOP-5 RISKS: GC roots shim / typed-OSR (net-new) / true deopt
  (highest risk, nothing in pon) / IC version-bump audit / feedback-slot
  stability. 02.05 SPLIT CANDIDATE.

## F-string bug hunt (parallel track) - COMPLETE (committed 0992ee9)
- BUG-1 debug forms + FINDING-3 walrus found; corpus slice 35+105 pinned;
  gate 107/107.

## Fix leaf (BUG-1 + FINDING-3) - COMPLETE (committed 075b453)
- debug_text through IR + dump; walrus bind_definite (+8 tests -> 72).

## Leaf 02.01 - ap-obj core values - COMPLETE (committed a5d6ab9)
- enum-of-16B Value, smi i61 + num-bigint (floor-normalized divmod),
  byte-exact hashing (int 2^61-1 / frexp float / siphash13 latin1-small-str /
  tuple xxHash / frozenset shuffle), compact dict w/ perturb+tombstones,
  key collapse, 48 tests (25 live-differential).
- Flagged: frozenset type -> 02.03; py_float_repr dedupe cleanup.

## Test leaf T-05 - COMPLETE (verified, pre-staged)
- 199 suite tests / 45 corpus cases pinned; 192 pass / 7 xfail pre-bin.

## Leaf 02.02 - tier-0 executor - LANDED, IN REVIEW-FIX (fix agent running)
- ~4300 lines, 8 modules: interp, builtins(37), exc hierarchy, PEP3101 format
  subset, frames/cells, iterator protocol, generators.
- Corpus at review time (orchestrator-verified): hello+types 2/2, fstrings
  33/35 (2 known out-of-scope), values 43/47 (V048/V050/V066 = classes 02.03;
  V016 = Rust char model vs surrogate escapes), stdlib 0/48 (02.08 surface).
  T-05: 192 pass / 6 XPASS / 1 xfail.
- ORCHESTRATOR STEER (mid-run) fixed: generator resume, iter/gen id collision
  (2-iteration cap root cause), traceback line numbers.
- FRESH-SMOKE FINDINGS (orchestrator, post-completion) -> fix agent dispatched
  (deleg_40b59065): BUG-A module-fn lookup from nested frames (self-recursion
  + mutual NameError; global VALUE reads fine, fn-name read from inner frame
  dies); BUG-B escaping closures lose cells (mk(10)(5) UnboundLocalError while
  immediate-call closure works - cell tied to maker frame lifetime?);
  BUG-C yield from (implement delegation; document return-value choice).
- Baseline for fix comparison: fstrings 33/35, values 43/47, hello 2/2.

## Leaf 02.02 - tier-0 executor - COMPLETE (committed e532638)
- All fixes verified firsthand: 24/24 differential tests green, 5/5 fresh
  smoke probes byte-match CPython (recursion, mutual, curried closure,
  shared-cell nonlocal factory, yield from). Rust 146/0, pytest 1223/0
  (s005 known), corpus baseline held: fstrings 33/35, values 43/47, hello 2/2.
- Notable root causes for the record: module-fn defs bound via store_local
  (module frames keep locals empty -> invisible); cell-classified params
  never re-seated into cells; frees not propagated through intermediate
  scopes (CPython symtable rule); nonlocal cell wiring omitted.
- KNOWN/QUEUED: debug-build stack overflow ~93 chained calls (02.05 item -
  run interpreter on worker thread or bump stack); V016 surrogate escapes
  (str-model follow-up); match class-patterns (02.03).

## STATUS UPDATE (orchestrator, 02.03 + 02.08-lite REVIEWED + committed 152a556)
- Both leaves verified firsthand (corpus rates reproduced) and committed.
- Spec Amendment A1 applied to docs/spec/05: base64/binascii/hashlib/random
  moved INTO v1 (native impls void the C-extension rationale of S-05-19);
  secrets stays OUT. Ids preserved per C-32.
- 02.04a (import machinery + vendored pure-stdlib) DISPATCHED per
  docs/02-04a-import-machinery-design.md. Vendor set = non-native modules
  only (native registry wins); metaclass/descriptor stress expected and
  wanted - fixing interp bugs it surfaces is the gate.

## Leaf 02.04a - import machinery + vendored stdlib - COMPLETE (committed 06df71b)
- importer.rs ~700 lines: full resolution order, circular visibility,
  packages/__path__, relative imports, byte-exact ImportError texts.
- vendored_stdlib/: 202 files (PSF-2.0).
- Gates: circular PASS, package+relative PASS, ImportError texts PASS.
- PATHLIB/DATACLASSES/ENUM: import OK, crash ON USE - "Refusing to create
  an inheritance cycle" from 02.03 MRO builder (pathlib PurePosixPath
  chain false-positive); unittest blocked on missing _py_warnings native
  module. -> fix pass dispatched (deleg_a82e46f4): BUG-A MRO cycle,
  BUG-B set.discard method routing (V044 regression: methods.rs arm
  exists but 02.03 attr protocol intercepts method-call attr lookups on
  builtin values first - values 46->45), BUG-C _warnings shim.
- stdlib slice 8/48 held; import machinery verified firsthand.

## QUEUE
1. Fix pass running (MRO cycle, method routing, _warnings).
2. 02.04 GC (MMTk swap).
3. 02.05 tier-1 (SPLIT per risks; + debug stack-depth).
4. 02.08-full: re engine, complex, class-method surface, metaclass hooks.
5. Plan 04 packaging (uv) -> T1 scrapy attempt.


## Leaf 02.03 - classes/protocols - LANDED (in review)
- ~1800 lines: ap-obj/classes.rs (TypeObject, C3 MRO w/ byte-exact errors, TypeFlags),
  ap-run/classes.rs (exec_class_def, attribute protocol S-03-09..12, descriptors
  property/classmethod/staticmethod/user + slot member descriptors, super() staged-ctx model,
  isinstance/issubclass w/ metaclass hooks + union marker, instantiate w/ S-03-05..08 arity
  + qualify), interp.rs wiring (class stmt, With __enter__/__exit__ + suppress, dunder
  dispatch binop/unary/compare/iter/contains/len/bool/repr/str, real exception classes as
  TypeObject-backed UserClass w/ class_ref back-refs, raise-from-class instantiates,
  identity-preserving except-as, __class__ cell seeding).
- Differential: crates/ap-run/tests/classes_differential.rs — 29/29 green (lookup order,
  MRO diamond, super chase/classmethod/validation, new/init arity+return, reflected ops,
  augmented, unary, iter/in/call/len/bool, user exception hierarchy + identity + targets,
  slots, dunder bypass, descriptors, repr/str, enter/exit, metaclass attr chain,
  isinstance tuple+union+errors).
- Corpus: values 46/47 (V016 surrogate stays red, documented), fstrings cases 34/35
  (f33 SyntaxError-shape, pre-existing), classes 27/33, exceptions 14/15, hello+types 2/2.
- name_resolution_differential 24/24 kept green; classes_differential 29/29; full pytest
  sweep 718 passed / 0 failed (51 XPASS all pre-existing Plan-01 seam xfails).
- Fixed en route (cross-file, flagged for orchestrator): builtins.rs builtin_sum drained
  iterators twice + returned float for all-int sums (f29/gate_gen_list); builtins_modules.rs
  register_shared_iter used a dead thread-local table (iter handles never resolved);
  builtins.rs call_builtin_ctor `?` on Rc deref; builtins_modules items() iteration in test cfg.
- NOT implemented (honest gaps): full metaclass hook execution (__prepare__/__new__/__init__
  bodies of custom metaclasses, c030/c031/c032 partial), metaclass-conflict TypeError for
  custom metas, mappingproxy immutability (C.__dict__ returns live dict, c002), vars() on
  instances (c018 - builtin lives in 02.08-lite surface), exception reprs inside containers
  (e008 line2), class kwargs -> __init_subclass__ delivery (c031 lower half).

## Leaf 02.08-lite - native stdlib modules + import registry (IN PROGRESS, uncommitted)
- builtins_modules.rs (new, mine): registry + math/json/itertools/operator/heapq/bisect/
  functools(reduce,partial)/string/textwrap/collections(deque)/os/os.path/posixpath/genericpath/
  stat/errno/keyword/platform/time/atexit/linecache/tempfile/io/random(MT19937-exact)/hashlib/
  base64/binascii; stdlib slice 0/48 -> 8/48 (s006 s007 s032 s039 s040 s041 s043 s048).
- interp.rs hooks: import registry (Stmt::Import/ImportFrom), iterable_of Iterator pass-through,
  attr_get native-attr + exception-attr, attr_get_public shim, __file__ global, atexit drain,
  is_builtin_exc_name JSONDecodeError, class_issubclass legacy-Type fallback (~8 small hooks).
- builtins.rs: dotted-marker dispatch, 11 introspection builtins (hasattr/getattr/setattr/
  delattr/dir/vars/id/callable/issubclass/ascii/open), compensated sum(), custom-repr in
  py_str_of/repr, NativeIter registration. iter.rs: IterState::Native delegate.
- exc_hier.rs + classes.rs: JSONDecodeError parent edges. Deps: md-5/sha1/sha2/libm.
- tests/stdlib_modules_differential.rs: 19 probes vs live python3.14 - all green.
- SPEC CONFLICT for orchestrator: random/hashlib/base64/binascii implemented per task brief
  but S-05-19/S-05-20 mark them OUT-of-v1 (fail-loud). No corpus case imports them; needs a
  spec amendment decision.
- Remaining stdlib slice blockers (taxonomy): s010/s011 re (real regex engine needed),
  s013/s014/s015/s016/s024/s025/s026/s027/s028/s030/s031/s033/s034/s038/s042/s044/s045/s046
  (classes/methods/With surface = 02.03), s001/s002/s005/s008/s009/s017/s021/s022/s023/s028/
  s036/s037/s047 (out-of-scope module bodies: sys surface gaps, codecs/struct/traceback/
  importlib/types/enum/dataclasses/typing/inspect/ast/tokenize/contextlib/copy/weakref/
  warnings/threading/argparse/pathlib/fnmatch/glob — each IN-v1 per spec 05 but not yet in
  the 02.08-lite slice), s012/s003/s004/s029 residual method-surface gaps (bytes.strip,
  complex, Partial-method kwargs).
