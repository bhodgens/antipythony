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

## 02.04a COMPLETION MESSAGE ADDENDUM (agent worked past my snapshot commit)
- Final state IS committed (worktree clean, 200 rust tests green at HEAD).
- Its final pass added: metaclass __new__ dispatch + type.__new__ builtin,
  metaclass-method binding (ABCMeta.register), object virtuals, exception
  __traceback__, star-imports, globals/locals/reversed/bytearray builtins,
  zip() id-collision fix, 19 interp bugs fixed total (documented in
  PROGRESS-02-04a.md).
- Vendored: 34 targets / 58 files (dropped encodings/ + importlib/metadata
  as C-backed). NOTE: my earlier "202 entries" count = files+dirs; 58 .py.
- VERIFIED: rust 200/0 at HEAD.
- STILL RED (new leaf 02.03b needed - metaclass __new__ w/ dict-derived
  instances): pathlib USE (MRO cycle false-positive - in running fix pass),
  enum member machinery (_member_names on dict), dataclasses, unittest
  (blocked: _py_warnings shim - in running fix pass).
- Harness note: t09/t10/t11 self-comparison fails under `uv run pytest`
  (venv python 3.14.4 shadows pinned 3.14.7) - pre-existing, use
  /opt/homebrew/bin/python3.14 -m pytest.

## QUEUE (updated)
1. Fix pass running (MRO cycle, method routing, _warnings) - deleg_a82e46f4.
2. NEW 02.03b: metaclass __new__ dict-instances (enum/dataclasses gates).
3. 02.04 GC (MMTk swap).
4. 02.05 tier-1 (SPLIT per risks; + debug stack-depth).
5. 02.08-full: re engine, complex, class-method surface, metaclass hooks.
6. Plan 04 packaging (uv) -> T1 scrapy attempt.

## Leaf 02.03b - metaclass __new__ dict-instances + warnings shim - LANDED (uncommitted, agent run)
- ENUM GATE GREEN (byte-exact vs python3.14): `import enum; class
  Color(enum.Enum): RED=1; GREEN=2; print(Color.RED.name, Color.RED.value,
  list(Color))` -> `RED 1 [<Color.RED: 1>, <Color.GREEN: 2>]`. Chain of
  root causes fixed:
  1. exec_class_def staged the metaclass-__new__ super ctx at the RESOLVED
     meta (M2) instead of the DEFINING class (Meta) -> super().__new__
     rebound Meta.__new__ to M2 (5-args TypeError). Fixed: BoundMethod
     channel with owner = defining class.
  2. type.__new__ rejected builtin-Type bases (class IntEnum(int, Enum)) ->
     route through base_type_ref synthesis.
  3. Classmethod __prepare__ never dispatched (is_user_function filter
     skipped descriptors) -> unwrap classmethod.
  4. EnumDict namespace: dict-subclass instances now carry a real BACKING
     dict (attrs["__ap_dict__"]); __prepare__'s instance rides the class
     frame (Frame.class_ns_inst) so body STORES route through its
     __setitem__; super().__setitem__ reaches the backing via
     __ap_dict_*__ virtuals; item/get/setdefault/pop/clear/contains/iter
     route to the backing.
  5. type() 3-arg + kwargs -> classes::dynamic_type (metaclass resolution +
     user meta __new__ + set_name hooks). CLASS DECORATORS now apply
     (exec_class_def step 10) - @_simple_enum depended on this.
  6. metaclass __call__ forwards ctor args (EnumType.__call__(cls, value));
     metaclass __getitem__ (Color["GREEN"]); C.__new__ object.__new__
     read allocates the RECEIVING class; str/int __new__ value-return shim;
     _value_/__init__ on builtin-value members; set.pop/remove/clear;
     dict get/setdefault/pop/clear.
  - BUG (interp): kw-only params after *args stole positional slots
    (def f(*a, b)) -> binder split into positional-consumable + kwonly.
  - BUG (interp): Flow::Break raised through a nested `with` body became
    LoopOut::Escape -> break-inside-with + for-else silently unwound the
    script (this is what killed every 'always' warning). Mapped to
    Break/Continue.
  - BUG: instantiate()'s extra super_stack pop removed the ENCLOSING
    frame's ctx after any nested instantiation.
  - AUGASSIGN attr read falls back to plain attr_get when the protocol
    misses (module attrs: _wm._filters_version += 1).
  - Module-attribute writes via attr_set_protocol.
  - Namespace/re shims: native re (literal engine: compile/match/search/
    escape/I-flag; re.Pattern.match/search markers), _colorize, codeop,
    _codecs (lookup/encode/decode/BOM_UTF8/lookup_error), _tokenize,
    sys.stderr/stdout write() markers, sys._getframe stub with
    f_code.co_filename + real f_lineno (outermost frame line).
  - map()/filter() builtins (eager v0).
- WARNINGS GATE GREEN (byte-exact): import warnings; simplefilter('always');
  warnings.warn('hi') matches CPython stderr format and exit code; the full
  warnings chain (filters, registry, WarningMessage, _showwarnmsg_impl)
  runs on the vendored _py_warnings.py.
- UNITTEST PARTIAL: imports now complete (needed the codecs/_colorize/
  codeop/_tokenize stubs + collections.namedtuple shim [3-arg type() class
  with positional-into-attrs instantiation] + map/filter). TestLoader/
  TestSuite path still fails: a __new__ 4-vs-5 arg bug inside the suite
  construction chain (unresolved; next leaf).
- PATHLIB PARTIAL: imports complete; PurePosixPath instantiation still
  hits a __new__ 4-vs-5 arg error in the class chain (unresolved; likely
  same root as unittest).
- Probes: crates/ap-run/tests/metaclass_enum_differential.rs - 6
  differential tests vs python3.14, all green (enum member gate, enum
  call/subscript, metaclass-new dict class, warnings warn+surface,
  dict-subclass namespace).
- Gates: values 46/47 (held), fstrings 34/36 (2 pre-existing), stdlib
  8/48 (held), overall 148/235 (baseline at HEAD on the same corpus:
  147/235; types slice moved 2->3 with the dict-namespace work). Rust
  211/0 (was 200; +6 new differential +5 unittest-path support), pytest
  1272 passed / 0 failed. cargo fmt clean. NOT COMMITTED per protocol.



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

## Fix pass (BUG-A/B/C) - COMPLETE (committed b1c3d67)
- BUG-A root cause NOT MRO: value_issubclass queried wrong metaclass w/ (a,a)
  instead of (type(b), a) - ABC register cycle-check false-fired. Fixed per
  CPython semantics. Plus coroutine type-name fastpath fix, str %-formatting
  implemented, set method routing fixed.
- values 46/47 restored (V044 green). pathlib progressed past cycle to a real
  metaclass-__new__ arity gap. unittest import still red (shim incomplete).
- Agent hit attempt-scope limit; report partially in Chinese (model artifact)
  - content decipherable, root causes verified by my own probes.

## Leaf 02.03b - metaclass __new__ dict-instances + _warnings + pathlib - DISPATCHED
- Gates: enum member machinery, unittest run w/ byte-exact output, pathlib
  Path('.') + PurePosixPath repr.
- This is the last blocker before GC/tier-1/packaging on the critical path.

## Leaf 02.03b - COMPLETE (committed e4864de)
- enum gate GREEN byte-exact: EnumType.__new__ dict-return becomes the class;
  member tracking; Color.RED.name/.value/list(Color) + Color(1) lookup.
- warnings gate GREEN: vendored _py_warnings chain runs; CPython-format
  UserWarning emitted.
- VERIFIED firsthand: enum probe byte-identical vs python3.14; warning text
  format matches.
- PARTIAL (queued): unittest + pathlib both fail on the SAME remaining bug:
  metaclass __new__ arity (TypeError: takes 4 positional args but 5 given)
  on subclass chains. -> fix leaf dispatched (deleg_296459a7).
- KNOWN GAP: Color(1) / Color["GREEN"] return address-repr (masking gap).
- rust 211/0 at commit.

## QUEUE
1. Metaclass __new__ arity fix (RUNNING deleg_296459a7) - flips
   unittest+pathlib gates.
2. 02.04 GC (MMTk swap).
3. 02.05 tier-1 (SPLIT per risks; + debug stack-depth).
4. 02.08-full: re engine, complex, class-method surface, metaclass hooks.
5. Plan 04 packaging (uv) -> T1 scrapy attempt.

## Post-02.03b cleanup (commits 43b7e4c, dc16788)
- Duplicate delivery of 02.03b confirmed no loss (worktree clean at HEAD).
- gen_cases.py (corpus GENERATOR, not a case) moved to tools/ - it was
  polluting the fstrings glob count (34/36 vs 34/35 confusion resolved:
  cases/ glob = 35 cases, 34 pass, f33 pre-existing).
- Running: arity fix (deleg_296459a7) - flips unittest+pathlib.

## Metaclass __new__ arity fix (fix leaf, 2026-10-18) — 4 GATES GREEN
- ROOT CAUSE: CPython treats user `__new__` as an IMPLICIT STATICMETHOD
  (type.__new__ wraps it) — `super().__new__` resolves UNBOUND and callers
  pass mcs/cls explicitly (ABCMeta: super().__new__(mcls, ...)). Our super()
  scan arms BOUND it like a normal method, then the caller passed cls again
  → "takes 4 positional arguments but 5 were given". Same class of bug at
  class-attr level (M.__new__ bound on read) and instance __new__ chains.
- FIXES (interp/classes): (1) zero-arg super() marks the super VALUE
  (__ap_unbound_new__) and attr_get returns the bare function for __new__;
  (2) comp frames now seed enclosing-function plain locals (enum._convert_'s
  `filter` param was invisible inside the members comprehension);
  (3) metaclass-level PROPERTIES now invoke on the class read
  (EnumType.__members__ → len(cls.__members__) fix); (4) unset-slot reads
  raise AttributeError instead of calling the None fget (pathlib lazy-init
  try/except AttributeError); (5) int.__new__-style arity: type.__new__
  (builtin) rejects leftover **kwds per CPython __init_subclass__ forwarding.
- SUPPORT (needed by unittest/pathlib import chains): new native module
  shims _io/_imp/_signal/_warnings/marshal/_opcode/_ast/_collections +
  posix/OrderedDict/defaultdict/ChainMap/Counter/UserDict surfaces;
  vendored_stdlib additions (PSF): difflib pprint signal ast dis opcode
  _opcode_metadata annotationlib(SHIM, t-strings out of grammar — documented)
  importlib/_bootstrap{,_external}; inspect.py two-line documented correction
  (_static_getmro/_get_dunder_dict_of_class expressed directly — no
  descriptor objects for type.__dict__ reads in v0).
- MORE FLIPPED EN ROUTE: str.format (+{!r}/{!s}), str.startswith/endswith
  tuple prefixes, str.isupper/islower/isalpha/isdigit/isalnum/isspace/istitle,
  int.to_bytes/bit_length, int.from_bytes classmethod, slice() constructor,
  bytearray as TYPE, re.sub + Pattern.sub (literal engine), os.PathLike.
  register, itemgetter(slice(...)), function/property __doc__/__dict__,
  module.__dict__, sys.builtin_module_names/base_prefix/prefix,
  sys.modules['__main__'] seed, enumerate(x, start) eager fix (was leaking
  the raw inner iterator), object.__subclasshook__ → NotImplemented,
  NotImplemented `is` identity, dict-comp nested-clause iter eval in the
  comp frame, isinstance subclasshook chain, itemgetter multi-idx marker
  fix (was yielding ('a','a') for itemgetter(1,0)).
- GATES (all byte-exact vs python3.14): import unittest=ok;
  pathlib.Path('.')→'.'; PurePosixPath('/a/b')→'/a/b';
  repr(PurePosixPath('/a/b'))→"PurePosixPath('/a/b')".
- DIFFERENTIAL: crates/ap-run/tests/metaclass_arity_differential.rs — 10
  probes (0/2/3-base chains, **kw, super() chains, type.__new__ direct,
  unittest+pathlib gates) all byte-exact green.
- CORPUS (flat vs pre-fix baseline, no regressions): values 46/47 (V016
  pre-existing), fstrings 34/35 (f33 pre-existing), classes 27/33,
  stdlib 8/48 (queued: json/os-full/io surface — 02.04a territory).
  IOError/EnvironmentError aliases added (stdlib s001 progresses past
  them; next blockers are inspect/attr surfaces, not class machinery).
- rust workspace: 212/0 green (41 suites); fmt clean. NOT COMMITTED
  (per protocol — orchestrator reviews + commits).

## Arity fix leaf - COMPLETE (committed 1ba8aa6)
- ROOT CAUSE (subtle, important): CPython wraps user __new__ as an IMPLICIT
  STATICMETHOD (type.__new__ wraps it) - super().__new__ resolves UNBOUND and
  callers pass cls/mcs explicitly. Our runtime bound it like a normal method,
  then the caller passed cls again -> 4-vs-5 arity TypeError. Three sites:
  super() scan arms, class-attr metaclass reads, instance chains.
- VERIFIED firsthand: unittest imports+prints, str(pathlib.Path('.'))=='.',
  repr(PurePosixPath('/a/b')) byte-exact. rust 212/0. Corpus held
  (values 46/47, fstrings 34/35, stdlib 8/48).
- RECORD for future leaves: __new__ is implicitly static in our runtime now;
  any new metaclass dispatch must preserve this.

## QUEUE
1. 02.03c (optional): re-run stdlib slice to count pathlib/enum-unlocked
   flips; Color(1) member repr masking.
2. 02.04 GC (MMTk swap).
3. 02.05 tier-1 (SPLIT per risks; + debug stack-depth).
4. 02.08-full: re engine, complex, class-method surface, metaclass hooks.
5. Plan 04 packaging (uv) -> T1 scrapy attempt.

## Leaf 02.08-full part 1 - method surfaces - DISPATCHED (deleg_5fadfe7b)
- str/bytes/dict/list/set/tuple method completion + int/float extras.
- Target: stdlib slice 8/48 -> 20+/48 (method-surface failures = ~24 of 40).

## Leaf 02.08-full part 1 - method surfaces - COMPLETE (uncommitted, agent run)
- stdlib slice 8/48 -> 16/48. Flips: s001 s002 s003 s004 s012 s046 s047
  (plus baseline s006 s007 s009 s032 s039-s041 s043 s048 held; s009 newly
  green, s032 held).
- METHOD SURFACE (methods.rs + builtins_modules call_builtin_values, kwargs
  now flow): str rsplit ordering fix (was reversed), split(None,maxsplit)
  remainder fix, splitlines universal-newline rewrite, casefold/swapcase/
  center/zfill/ljust/rjust/rpartition/expandtabs/translate/str.maketrans/
  isascii/isidentifier/isprintable/isnumeric/isdecimal/rindex/format (full
  PEP 3101: auto/manual/named fields, !r/!s/!a conv, format-spec via
  format::format_value), startswith/endswith [start,end]; bytes
  strip/lstrip/rstrip/split/rsplit/join/replace/find/rfind/index/rindex/
  count/startswith(tuple)/center/zfill/title/upper/lower + bytes() ctor
  (int zero-fill/iterable/str+encoding); dict update(dict/iterable/kwargs),
  fromkeys (type+instance), real dict_keys/Items/Values view objects
  (repr, iteration, LIVE back-pointer so v=d.items(); d[3]=4 sees inserts);
  set/frozenset union/update/intersection(+_update)/difference(+_update)/
  symmetric_difference(+_update)/issubset/issuperset/isdisjoint/copy w/
  multi-arg; tuple count/index(+start/end); int.from_bytes (unbound
  classmethod data-slot handling) + bit_count; float
  as_integer_ratio(bit-exact)/hex/is_integer; complex as Opaque pair
  (repr/real/imag/arith/truthiness - V047 bool(0j)=False preserved).
- UNBOUND builtin-type methods: list.copy(x)/dict.get(d,k)/str.upper(s)/
  set.union - Type-attr markers route through call_native tags
  (str/list/dict/set/tuple/bytes/int/float); copy.py unblocked.
- EN ROUTE flips: compile()/__import__ builtins, enumerate(start=kwarg),
  eval SyntaxError msg pin ("invalid syntax"), ascii \xNN short form,
  dir(int)/dir(list) surfaces for interned Types, IOError/EnvironmentError
  is-OSError identity (real exc classes re-aliased post-shadow),
  sys.version_info subscript/<values>, sys 15 name attrs
  (orig_argv/meta_path/hexversion/...), os 13 name attrs + os.path 5,
  io 3 class names + DEFAULT_BUFFER_SIZE=131072 + text-mode StringIO
  (str payloads, line iteration, write-mode read raises
  UnsupportedOperation) + TextIOWrapper.encoding, json JSONEncoder/
  JSONDecoder classes + cls=default() hook + view objects, itertools
  count type name, tempfile.TemporaryDirectory as real class
  (with-block, cleanup), importlib native import_module/reload/find_spec
  + _abc/_bootstrap/_util/machinery shims, SyntaxError .msg attr.
- VENDORED (documented divergences, PSF headers in place): _collections_abc
  builtin-type register() calls wrapped try/except RuntimeError (the
  legacy name-Type model mis-fires the cycle probe - CPython does these in
  C); that un-breaks weakref/contextlib/traceback/unittest imports.
- DIFFERENTIAL: crates/ap-run/tests/method_surface_differential.rs - 34
  probes vs live python3.14, all green.
- NOT IMPLEMENTED (honest gaps): bytearray mutation (bytes/bytearray
  conflation in the v0 value model; needs a distinct mutable repr),
  dict-view set operators (&|^-) and view dynamism beyond iteration,
  numbers/struct/threading/locale/encodings modules (whole-module work,
  part 2), inspect/ast/tokenize/typing/dataclasses/pprint surfaces.
- GATES: stdlib 16/48 (8->16), values 46/47 (held, V016 pre-existing),
  fstrings 34/35 (held), classes 27/33 (held), exceptions 15/15 (held).
  rust workspace 246/0 across 42 suites (was 212; +34 differential),
  pytest 1272 passed/0 failed. cargo fmt clean. NOT COMMITTED per protocol.

## Leaf 02.08-full part 1 - COMPLETE (committed c298962)
- stdlib 8/48 -> 16/48 (below the 20 target - remaining 32 need whole modules:
  re engine, complex, importlib file machinery flips, io surfaces).
- str rsplit ordering BUG fixed (was reversed). kwargs flow through
  call_builtin_values. +34 differential (246 rust total). All gates held.
- Taxonomy of remaining 32: re-dependent, complex-number, importlib-file
  cases (pathlib-adjacent), deeper io.

## QUEUE
1. 02.04 GC (MMTk swap) - NEXT
2. 02.08-full part 2: re engine (biggest single unlock), complex numbers
3. 02.05 tier-1 (SPLIT per risks; + debug stack-depth)
4. Plan 04 packaging (uv) -> T1 scrapy attempt

## Leaf 02.04 - GC - DISPATCHED (deleg_4f025323)
- RESCOPED (orchestrator decision): Stage 1 = Rc-cycle collector + gc module
  (Python gc semantics NOW, unblocks gc/weakref corpus cases). Stage 2 = MMTk
  swap DESIGN DOC + compile spike only (no migration this leaf). MMTk swap is
  the tier-1 perf story and pairs with 02.05's root-map work.

## Leaf 02.04 GC - FIRST AGENT DIED (provider network failure at ~90min)
- Worktree salvage: compiles clean, gc.collect() returns counts, but
  SEMANTICALLY BROKEN: live enum/unittest classes get swept (NameError:
  enum_class) - root-scoping bug in mark_reachable/count_internal_refs.
- My interim patches: try_borrow + SCAN_CONTENDED abort in collect() (fixes
  a RefCell reentrancy panic in scan_obj Obj-arm).
- CONTINUATION DISPATCHED (deleg_f5fd79fd): fix root-scoping, all gates,
  + MMTk design doc (stage 2). My interim RefCell patches kept.

## 02.04 continuation agent ALSO stalled/died (~50min quiet, no procs)
- Its salvage was real: enum gate now PASSES (reverted a wrong scope_path
  fix; the NameError was scope misclassification, NOT the collector).
- weakref bridge dep-graph wired. Committed salvage a9a43e6.
- REMAINING: weakref.WeakKeyDictionary surface (blocks unittest),
  gc.collect() count semantics (67 vs 114 - objects vs cycles), MMTk doc.
- COMPLETION LEAF DISPATCHED (deleg_8252f8c2) with all four remaining items.

## Leaf 02.04 completion - weakref surface + unittest gate + gc count semantics - COMPLETE (uncommitted, agent run)

### Weakref surface (WeakKeyDictionary et al)
- Native `weakref` module (wins the registry over vendored weakref.py) now
  carries the full Python surface natively over the token machinery:
  WeakKeyDictionary / WeakValueDictionary / WeakSet (Opaque-backed classes;
  data dict/set holds ref TOKENS keyed by target address) + WeakMethod
  (watching the bound method's receiver) + working proxy (transparent attr
  forwarding via the interp attr layer, ReferenceError on dead target).
- ap-obj: weakref tokens now hash/compare BY TARGET (`weakref_target_addr` in
  value.rs; hash.rs + dict.rs values_eq arms) — two refs to the same object
  dedupe in sets/dicts, which the WeakSet/WKD data model requires.
- Design note: dead entries are PRUNED AT ACCESS (len/iter/get/contains);
  CPython prunes at death time via the `_remove` callback. Observable
  difference is timing-only, both agree after gc.collect() (documented in
  gcsupport.rs header, C-22 note).
- module surface: ref/ReferenceType (same Type value; KeyedRef alias),
  ProxyType/CallableProxyType/ProxyTypes (real Types; type_matches arm for
  `_weakref_proxy`), WeakKeyDictionary/WeakValueDictionary/WeakSet/WeakMethod
  class values, getweakrefcount/getweakrefs (new ap-gc accessors
  weakref_count_for/weakref_tokens_for), finalize (existing).
- Slotted instances (no instance dict) are NOT weakrefable (s034 NonRef pin).
- gc015 finalize surface fixed: `f.detach()` was raising AttributeError — the
  BoundMethod marker dispatch for tokens now routes detach/__call__ through
  gcsupport (finalize_detach/finalize_call).
- GATE: tests/corpus/stdlib/s034_weakref_basics.py byte-exact vs python3.14.

### unittest gate
- Chain blockers fixed in order: WeakKeyDictionary (above), locale native shim
  (gettext._expand_lang normalize; gettext chain from argparse), shutil native
  shim (get_terminal_size for argparse HelpFormatter), _colorize
  decolor/colorize + argparse/unittest theme sections (empty-string no-color
  fields), OSError family exceptions (FileNotFoundError et al in classes.rs
  EXC_TREE + exc_hier parent_of), type.__flags__ = 0 (inspect.isabstract),
  functools.cmp_to_key (opaque wrapper; sorted/list.sort order via the parked
  cmp — CPython K-wrapper semantics), dir(UserClass) over the MRO dicts
  (getTestCaseNames discovers test methods through dir), callable() true for
  instances whose class defines __call__ (suite.addTest probe), exception
  chain dunders (__cause__/__context__/__traceback__/with_traceback) on both
  legacy Exception and real-class instances.
- RUNTIME BUGS fixed en route (all cross-file, flagged):
  1. method-frame closure wiring: class-body frames carried scope_path="" so
     methods registered as ".<locals>.m" (leading dot, no class segment);
     scope_of missed and nested defs in methods lost their free-variable
     cells (argparse's _metavar_formatter was the canonical victim). Fix:
     class frame scope_path = parent + "." + class name, matching the
     lowering's open_scope keys. This is the "method-scope classification"
     leaf the stalled prior agent reverted for — the earlier revert was the
     MISSING PARENT JOIN, not the idea; the enum regression it hit is now
     understood and covered (below).
  2. forward-classified cells: a scope.cells name not yet BOUND at param time
     (enum.EnumType.__new__'s `enum_class`) now aliases its cell at bind
     (None-arm in the rebind loop) — later stores land in the cell, so
     nested-scope readers (comps, inner defs) see the value. This is what
     makes the scope fix safe for enum (the prior agent's regression).
  3. GC-vs-interpreter re-entrancy: an auto-collect can fire INSIDE
     call_function (dict alloc in the super sentinel literal) and re-enter
     via a weakref callback; pending_super_ctx/super_stack borrows are now
     try_borrow (contended = an outer frame is staging) and the sentinel
     TypeObject is built BEFORE borrowing super_stack.
  4. py_str_of dispatches user __str__ (owner not object/Exception roots) —
     TestCase.__str__ is the canonical consumer; str(A()) with class
     __str__ now returns "A-str".
- VENDORED CORRECTIONS (documented AP-CORRECTION headers, same precedent as
  the inspect.py two-liner): unittest/case.py _Outcome.testPartExecutor
  rewritten from @contextlib.contextmanager (generator CM whose recording
  clauses can never fire under our eager generators) to a module-level
  _PartExecutor CM class (attribute-carried state; clause order preserved);
  unittest/result.py _exc_info_to_string renders the failure body directly
  (TracebackException needs real tb frames + StackSummary list-subclass —
  both v0 gaps). Honest gap: failure bodies lack the CPython traceback frame
  lines (no live tb chain); header/footer lines match.
- GATE: TestLoader/TextTestRunner with a 2-method class (1 pass/1 fail) —
  names, test_fn (test) lines, OK/FAILED (failures=1) shape, "Ran 2 tests"
  (timing masked). NOTE unittest.main()'s argparse leg still dies in the
  vendored re literal engine (_get_nargs_pattern regexes) — the loader/runner
  path is the gate; re engine (02.08-full part 2) unblocks main().

### gc.collect() count semantics
- collect() returns the count of SWEPT REGISTRY ENTRIES — the direct analogue
  of CPython's "unreachable objects" return (CPython's absolute number
  includes ITS startup garbage; ours ours — the magnitudes are not
  comparable across interpreters, only the definition is). Documented in
  gcmod.rs collect() and docs/gc-mmtk-swap.md §1.
- Alignments while pinning: explicit collect() resets get_count()[0] to 0
  (gc016 "after collect gen0 is 0"); collect(gen) threads the requested
  generation into the callbacks info dict (gc006 collect0/collect1 legs).
- GATE: gc corpus slice 9/17 (was 6 at leaf start; gc006/gc013/gc016 flipped
  this leaf). Remaining 10 need lazy generators (eager-gen snapshots pin
  value lists as permanent roots — gc013's genexpr case exposed this; fixed
  for the specific case by dropping exhausted iterators from the table) and
  threshold-realistic auto-collection timing — queue for 02.05.

### MMTk design doc
- docs/gc-mmtk-swap.md: Gc handle->MMTk reference mapping, MarkSweep-first
  binding choice + plan-flip path, root scanning via the pon shim pattern
  (shadow stack first, pon stackmap via 02.05), heap sizing + APX_MMTK env,
  weakref semantics on mmtk reference processing, 6 ordered migration steps
  each with a gate. Spike DEFERRED (network-isolated leaf; mmtk-core dep tree
  needed) — documented in §8.

### Differential
- crates/ap-run/tests/gc_differential.rs — 12 probes vs live python3.14
  (addresses/timing masked): cycle counts, ref liveness, unweakrefable
  matrix, WKD, WVD, WeakSet, WeakMethod, proxy (+ReferenceError), weakref
  count surface, unittest runner run, unittest assert surface, gc module
  surface. All green.

### Gates (orchestrator-verified at review)
- rust workspace 247/0 (incl. the new battery); fmt clean.
- pytest 1272 passed / 0 failed.
- Corpus: values 46/47 (held), fstrings 34/35 (held), stdlib 17/48 (16->17,
  s034 weakref flips), exceptions 15/15 (held), classes 27/33 (held),
  hello+types 2/2 (held), gc 9/17 (was 6).
- NOT COMMITTED per protocol.

## Leaf 02.04 - GC - COMPLETE (committed 81e2e79)
- weakref native surface (WKD/WVD/WeakSet/WeakMethod/proxy, token hash by
  target address); unittest gate GREEN (TestLoader/TextTestRunner,
  CPython-shaped output); arity battery green; gc.collect swept-entry
  semantics documented; MMTk design docs/gc-mmtk-swap.md (6 gated steps).
- rust 247/0; pytest 1272/0; corpus: values 46/47, fstrings 34/35,
  stdlib 17/48 (+1 s034), gc 9/17 (+3), classes 27/33, exceptions 15/15.
- Honest gaps: unittest failure bodies lack traceback frame lines (needs
  live tb chain); unittest.main() argparse leg needs real re engine;
  10 gc corpus failures need lazy generators (02.05); bytearray mutation
  needs mutable repr.
- PLAN 02 REMAINING: 02.05 tier-1 (SPLIT per risks) + 02.06 AoT + 02.07
  threading + 02.08-full p2 (re engine, complex, class-method deep).

## QUEUE
1. 02.08-full p2: re engine + complex + remaining method surface
2. 02.05 tier-1 (SPLIT per risks) + 02.06 AoT + 02.07 threading
3. Plan 04 packaging (uv) -> T1 scrapy attempt
4. Plan 03 WASM backend (after native is at T1 level)

## Leaf 02.08-full p2 - re engine - DISPATCHED (deleg_51dcd1ab)
- Full backtracking regex engine (byte-exact vs CPython: leftmost-first,
  backrefs, lookarounds; Rust regex crate REJECTED - its DFA semantics differ
  deliberately). ~50 differential probes; s010/s011 expected to flip.
- Post-02.04: unittest chain works (weakref/_warnings landed), so warnings.py
  filter patterns are now testable end to end.

## Leaf 02.08-full p2 - re engine - COMPLETE (uncommitted, agent run)

### ENGINE DESIGN CHOICE (documented per brief)
- HAND-ROLLED backtracking engine in Rust (crates/ap-run/src/sre.rs ~2500L +
  sre_tables.rs): hand-written recursive-descent parser -> AST ->
  CPS-style recursive backtracking matcher. Rust `regex` crate REJECTED
  (DFA semantics deliberately differ: no backrefs, no uniform lookaround,
  different alternation preference -> byte-exactness impossible). Vendored
  re/_parser+_compiler+`_sre` path ALSO REJECTED (needs array-module
  semantics, memoryview.cast, a bytecode VM - more interpreter surface for
  the same observable behavior). `re` stays a NATIVE module (registry wins
  over the vendored package; vendored re/ unused, documented).
- CPython semantics: leftmost-first match, greedy/lazy quantifiers with
  backtracking + zero-width-loop protection (one empty iteration then stop,
  CPython-observed), capture reset on backtrack, fixed-width lookbehind
  (byte-exact error), \1..\99 backrefs (+IGNORECASE forms), named groups
  (?P<..>/\g<..>), conditionals (?(1)y|n), atomic groups (?>..) +
  possessive quantifiers (a++ - implemented as atomic(greedy-repeat)),
  \b \B \A \Z, MULTILINE/DOTALL/IGNORECASE/VERBOSE/ASCII + inline
  (?i:..)/(?i)..-global/scoped (?-i:..) with CPython flag-conflict errors,
  full-Unicode \d \w \s (tables extracted live from CPython 3.14.7
  isdigit/isalnum/isspace over the whole codepoint space) + _casefix
  extra-cases set for IGNORECASE (STRASSE/ſ pairs).
- CATASTROPHIC-BACKTRACKING GUARD (documented divergence): 4M-step limit
  per match call raises re.error("ap: matching step limit exceeded...")
  where CPython would hang. No corpus/probe hits it.
- BYTES PATTERNS decode latin-1 into the str path (documented v0 gap:
  matches return str-typed groups). re.Scanner exists as a name surface.
- DOCUMENTED GAP: \N{...} supports <control-XX>/U+XXXX/C0 names only (the
  30k-entry unicodedata name table is not viable in v0).
- MODULE API: compile/match/fullmatch/search/findall/finditer/sub/subn/
  split/escape/purge + flags (incl. NOFLAG repr "re.NOFLAG" via a marker
  object int()ing to 0) + Pattern/Match types with isinstance + __all__ +
  __version__ "2.2.1" + re.error is re.PatternError (real class, module
  're'); kwargs (flags=/count=/maxsplit=) and positional slots per CPython
  signatures; compiled-pattern+flags ValueError byte-exact.

### VALUE LAYER
- Pattern/Match = Opaque objects; engine state rides thread-local identity
  maps (PAT_TABLE/MATCH_TABLE). Match surface: group([1,2..]|name),
  __getitem__, groups, groupdict (definition-ordered), start/end/span,
  expand, re/pattern/flags/groups/groupindex/string/pos/endpos/lastindex/
  lastgroup/regs; reprs byte-exact incl. CPython quote-flipping for
  embedded quotes. Pattern repr re.compile('pat', re.A|re.I) byte-exact.

### EN-ROUTE RUNTIME BUGS FIXED (cross-file, flagged)
1. for/while-ELSE ran unconditionally after break (Rust `loop { break }`
   fell through to the orelse block with no completed flag). Visible in
   _py_warnings' filter loop (warn ignored filters) - fixed for BOTH
   Stmt::For and Stmt::While (interp.rs).
2. del lst[a:b] unsupported -> 'list indices must be integers'
   (resetwarnings' `del _filters[:]`). Added Key::Slice arm to
   subscript_del + PyList::remove_range.
3. groupdict iteration order (HashMap -> grouporder vec, definition order).
4. ignorecase literals were not folded at parse (lit() now applied to bare
   chars in parse_sequence).
5. escapes \a \t \n \f \r \v were rejected; \x \u \U check ordering
   fixed (bad-escape arm ran before hex parsing).
6. {,n} parsed as literal '{' (CPython: {0,n}).
7. module-level re.match/search positional flags slot misread as pos/endpos
   (re_op_form splits module vs Pattern-method arg shapes).
8. PatternError added to the builtin exception tree leaked into
   Exception.__subclasses__ (e002/e014 pins) - excluded from
   subclasses_of while keeping issubclass (s011) + __module__=='re'.
9. stdlib exception classes report their home module
   (re.error.__module__=='re', JSONDecodeError 'json') via the class dict +
   Type-attr surface.

### GATES (all verified firsthand vs /opt/homebrew/bin/python3.14 -S)
- rust workspace 289/0 (was 247; +42 re_differential + trivial suite
  drift); fmt clean; pytest 1272 passed / 0 failed / 5 skipped /
  751 xfailed / 126 xpassed (baseline 1272/0).
- CORPUS: stdlib 17/48 -> 20/48 (s010 s011 s033 flip = the brief's exact
  targets; +1 more from the for/else fix landing on s0xx paths);
  values 46/47 (held); fstrings 35/35 (held); classes 27/33 (held);
  exceptions 15/15 (e002/e014 restored after the PatternError tree
  addition - was 13/15 mid-leaf); gc 13/17 (was 9 at leaf start; +4 from
  for/else + del-slice fixes, none of the remaining 6 regressed);
  hello+types 2/2 (held).
- WARNINGS CHAIN: simplefilter/ignore/always end-to-end through
  _py_warnings' filter loop (the for/break/else bug that silently ran the
  else clause is fixed); warn_explicit + resetwarnings (del lst[:]) green.
- UNITTEST: imports + TestLoader/TextTestRunner path green (failure-body
  traceback frames remain the documented v0 gap; 'Ran 2 tests' timing
  sign -0.000s vs 0.000s pre-existing).
- DIFFERENTIAL: crates/ap-run/tests/re_differential.rs - 42 probes vs live
  python3.14, ALL green (literals/classes/escapes, quantifiers incl.
  nested-repeat group semantics, alternation order, backrefs incl.
  unmatched-group failure, named groups + \g templates, conditionals,
  lookahead/lookbehind + variable-width-lookbehind error texts, atomic/
  possessive, flags I/M/S/X/A inline+compiled, match-object surface +
  reprs + regs + pos/endpos, module API incl. sub backref templates +
  callable repl, split empty-match semantics, findall group forms, escape,
  purge, types/isinstance, 10 compile-error texts, zero-width scan,
  pos/endpos, warnings/argparse pattern shapes, {,n}, casefold edge,
  multiline $, groupref-in-repeat, template errors, dot/newline).
- NOT COMMITTED per protocol. Sibling-agent files touched by their own
  leaves (gcmod.rs finalizer ordering, t08 bench_report) left untouched.

## Leaf 02.08-full p2 - re engine - COMPLETE (committed 1ebdf45)
- sre.rs ~2.6k lines: recursive-descent parser -> AST -> CPS backtracker.
  Rust regex crate REJECTED (DFA semantics differ -> byte-exact impossible).
- Full scope green: quantifiers incl. possessive/atomic, backrefs, named
  groups, lookarounds, flags, match objects, error texts; 42 differential
  probes (289 rust total); stdlib 17/48 -> 20/48 (s010/s011 flipped).
- fmt clean; corpus held elsewhere.

## SEQUENCE CHANGE (orchestrator, toward "compile target projects")
- 02.07 threading DISPATCHED BEFORE tier-1: T1 scrapy needs correct
  threading (twisted imports it), NOT tier-1 performance. Tier-1/AoT move
  AFTER T1 validation (they are perf/distribution, not correctness).
- GIL decision documented: v1 ships a GIL (blocking-op release pattern),
  diverging from the no-GIL vision; T120-T122 no-GIL probes expected
  red/masked and documented. no-GIL swap pairs with tier-1 root-maps later.
- Threading corpus pre-staged by test track: T101-T122 + s038 (19 cases).

## Post-re-engine corpus state (verified, 380f28e + dc16788)
- TOTAL 158/215 (73%): values 46/47, fstrings 34/35, stdlib 20/48,
  gc 16/17 (tuple-tracking fix +3), classes 27/33, exceptions 13/15
  (e002/e014 NEW regressions - exception-hierarchy repr diffs, queued for
  fix pass with threading), threading 0/18 (02.07 in flight).
- 02.07 threading RUNNING (deleg_3862ee37).
- Exception repr regressions -> fix pass after 02.07 lands.

## 02.04 continuation (2026-10-19, GC-semantics pass on the 02.07 worktree)
- Finalizer ordering now DETERMINISTIC (5/5 runs p0..p4; previously all
  permutations run-to-run): fix = per-collection garbage list snapshot
  (GARBAGE_NOW) in gcmod collect_gen; sweep_weakrefs keys off the current
  cycle's deaths instead of registry membership (which retain mutates after).
- weakref.finalize now FIRES: gcsupport fire_dead_weakrefs resolves the
  finalize dispatch marker through the FINALIZERS table before the
  WEAK_TOKENS lookup (gc015 was "after collect: [] alive: True").
- Address-reuse poisoning fixed: FINALIZED_ADDRS purge keyed on the
  reachable/resurrected sets each cycle (a swept addr reused by a new object
  used to inherit the dead object's "already finalized" mark — observed:
  whole blocks of instances created after an auto-collect never fired).
- unraisable routing fixed: write_stderr resolves `write` through the attr
  protocol (native StringIO keeps it in the methods table, attrs.get missed);
  sys.unraisablehook/__unraisablehook__ seeded (None = native default) and a
  None hook no longer diverts the report. Swapped-stderr capture 0 -> 197B
  (CPython 222; extra frame line = v0 coarse line map, documented).
- Unraisable report shape: qualified name via fn registry (__qualname__);
  raise-site line captured on the exception (exc.raise_line, Raise arm) —
  more accurate than the old caller-line cell, but the ordinal offsets
  differ from the last green run, so gc009's pinned line-27/raise-echo bytes
  still differ (16/17 held; the pin asserts engine-specific coarse-map
  output — revisit with the SourceMap follow-up, NOT a pin edit).
- Real __traceback__ chain on handler-bound exceptions (tb_frame.f_code.
  co_name / tb_lineno / tb_next walk) — tb7 probe matches CPython's frame
  list shape; unittest.result cleanup paths benefit.
- Gates on the shared worktree (values 46/47, fstrings 35/35, gc 16/17,
  classes 27/33, exceptions 13/15 — all at the documented envelope);
  rust workspace 44/44 suites 0 failed; gc_differential + arity + enum
  batteries green; heap_probe_10k fired 10000/10000 tokens complete.
- Root-scoping: NOT the collector — confirmed by the 02.04 salvage commits
  (a9a43e6: collector-disabled still reproduced; wrong scope_path assignment
  in exec_class_def was the culprit and is already fixed at HEAD).

## 02.07 threading (native OS threads, GIL v1) — LANDED (working tree, uncommitted)
- New: `crates/ap-run/src/threading.rs` (GIL + registries + worker bootstrap +
  exit protocol), `crates/ap-run/src/threading_call.rs` (Python-visible
  threading/queue surfaces). Wired through builtins_modules (module registry +
  call_native/native_attr routing), interp (Opaque repr/__class__/type(),
  GIL-retry on shared RefCells), classes (attr_set protocol arm),
  gcsupport (`GilSendPub` Send escape + worker GC-registration forwarding to
  the main thread's collector view).
- GIL v1 DESIGN (documented divergence from the no-GIL master vision): ONE
  global interpreter lock (`std::sync::Mutex`) around bytecode execution.
  Every blocking primitive (join/event-wait/cond-wait/barrier-wait/sem-
  acquire/queue get-put/sleep/file-io) releases the GIL BEFORE parking and
  reacquires on ALL exits — "GIL FIRST" invariant. RefCell state shared
  across worker threads (frame_stack, stmt_lines, lambda_bodies, iterators,
  the globals PyDict) is write-once or statement-atomic; contended borrows
  RETRY with a 50us spin instead of panicking (holders always run under the
  GIL, so a borrow clears within a scheduler quantum). gcmod REGISTRY stays
  thread-local with worker registrations forwarded to the main thread
  (global-lock-shares-the-heap option, matching the GIL; per-thread heaps +
  Weak bridge deferred to the no-GIL swap, 02.05-tier1 era).
- Surfaces: Thread (name/ident/is_daemon/native_id, start/join(timeout)/
  is_alive/run, daemon semantics: main exits waits non-daemons only, T107),
  Lock/RLock (reentrancy), Condition (wait/notify/notify_all/wait_for),
  Event, Semaphore/BoundedSemaphore (over-release error text), Barrier
  (parties/action/reset/BrokenBarrierError on timeout+abort), local (T106
  per-thread isolation), Timer (interval pre-wait, cancel; Timer IS-A Thread
  for issubclass), enumerate/active_count/current_thread/main_thread,
  excepthook + ExceptHookArgs (T109/T110), settrace/suspend hooks minimal,
  sys.setswitchinterval accepted+ignored (documented), TIMEOUT_MAX pin (G-3).
- queue module: Queue/LifoQueue/PriorityQueue over the same prims —
  empty/full/qsize/get/put/get_nowait/put_nowait/task_done/join; Empty/Full
  str() == '' byte pins.
- Default thread names `Thread-N (target)` CPython-3.14 form; MainThread
  class name via _MainThread; repr heads byte-exact (addresses masked per
  corpus convention).
- Corpus: threading slice 17/18 PASS under ap-conform differential vs
  python3.14 (T101-T108, T110-T115, T120-T122; s038_threading_smoke PASS
  byte-exact). T109 FAILS BY DESIGN — G-1: the pinned stderr embeds the
  reference's own threading.py bootstrap frames with absolute install paths
  (contract gap, xfail in the test track; NOT re-pinned).
  T120/T122 (no-GIL probes) PASS for correctness under the GIL — timing
  differs (T121 scaling gate is the 02.05-era work; the no-GIL swap remains
  the master vision).
- Test track: tests/t10_threading 97 passed, 11 xfailed (contract xfails),
  0 failed (44min wall — the suite runs the corpus differentially).
- Fixed en route: genexpr-at-module-scope comp-target store leaked the loop
  var into module globals (interp); contended-RefCell panics under real OS
  threads (touch_line/current_frame_line/last_executed_line/frame_stack
  push+pop/lambda_bodies/globals try_get+set retry); entry_for/registry
  borrows; Barrier timeout kw routing; Timer kwargs args=/kwargs=/function;
  double-start serial check under contention (try_borrow).
- NOT done (deferred): sys.setswitchinterval actual switching (v1 ignores),
  per-thread heaps (no-GIL swap), T121 >=3x scaling gate (needs tier-1),
  T109 G-1 resolution (spec/impl decision pending).

## Leaf 02.07 - threading - COMPLETE (committed faf7a87)
- threading corpus 17/18 (T109 = G-1 gap: reference install paths in pinned
  stderr - documented); s038 byte-exact green; T120-T122 pass correctness
  under GIL v1.
- PLAN 02 CORRECTNESS TRACK NOW COMPLETE. Remaining Plan 02: 02.05 tier-1
  (perf), 02.06 AoT (distribution) - both deferred until after T1 validation.

## NEXT: Plan 04 packaging - the LAST structural piece before scrapy T1
1. 04.01 uv subprocess wheel-fetch (pure-python wheels into build cache)
2. import integration: fetched packages join sys.path at compile
3. scrapy crawl file:// spider byte-compare vs CPython reference = T1 GATE

## Leaves 04.01+04.02 - packaging/uv - DISPATCHED (deleg_643778e4)
- antipythany add (uv subprocess, pure-wheel-only gate), site-packages
  cache + sys.path wiring, toml/lockfile manifest, e2e tests with six/
  packaging (byte-exact) + lxml fail-loud diagnostic.
- This is the LAST structural piece before the scrapy T1 attempt.

## 02.04 FIRST agent (deleg_f5fd79fd) duplicate delivery - NO ACTION
- Reported 28h after assumed-dead; its worktree got swept into later commits.
- All claims verified in HEAD: GARBAGE_NOW deterministic finalizers,
  finalize dispatch fix, address-reuse pollution fix, tb chain, MMTk doc.
- Root-cause confirmation: sweep bug was scope_path, not the collector
  (matches continuation agent's independent finding).
- gc corpus at HEAD: 16/17 (gc009 line-map pinning offset - documented).

## Leaves 04.01 + 04.02 - packaging - COMPLETE (committed 6f0b886)
- ap-pack crate (manifest/lockfile, uv wrapper, pure-wheel filter,
  content-addressed cache); CLI new/add/run end to end.
- MILESTONE: FIRST THIRD-PARTY PACKAGE RUNS - six 1.17.0 resolved via uv,
  imported, executed byte-exact under antipythany.
- +16 rust tests (362 total). cext diagnostic: lxml add fails loud.
- VERIFIED firsthand: new -> add six -> run main.py = "six works 1.17.0"
  (CPython w/o venv cannot even import it - our cache is self-contained).

## NEXT: T1 SCRAPY ATTEMPT
- Prereqs all in place: classes/imports/threading/re/gc/weakref/stdlib/
  vendored stdlib + packaging fetches pure-python deps (twisted et al).
- Remaining risk: scrapy's deep deps (queuelib/w3lib/parsel/protego/
  itemloaders/jmespath/cssselect - all pure) + twisted (pure, large).
- T1 gate: scrapy crawl file:// spider, items.jsonl byte-compare vs CPython.

## T1 SCRAPY ATTEMPT - DISPATCHED (deleg_353c29f1)
- The T1 gate: scrapy crawl on file:// spider, items.jsonl byte-compare vs
  CPython reference.
- Iterative structure: env setup -> dependency audit -> import-depth testing
  (twisted first) -> interpreter fixes in ap-run/ap-obj as bugs surface ->
  minimal spider run -> gap report docs/t1-attempt.md.
- Expect interpreter conformance bugs; fixing them IS the point.

## T1 completion leaf - DISPATCHED (deleg_1c33caaa)
- The 5 precise blockers from the attempt doc: importlib.resources.files
  (None-bind), chained-traceback frame attrs, 2-3-arg exec (twisted
  reactor), typing/attrs None-callable tail, abc.register semantics.
- Acceptance: the staged T1 gate - spider crawl w/ items.jsonl byte-equal
  to the CPython golden at /tmp/t1/ref/.
- 20 interpreter fixes already landed in the attempt (typing works,
  twisted top-level imports, module-level decorated defs, lru_cache(N),
  PEP 560 __class_getitem__, __init_subclass__ everywhere).
