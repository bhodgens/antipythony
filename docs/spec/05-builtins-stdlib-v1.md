# Spec 05 — Builtins and Stdlib v1

Part of the spec pack (00.05). Source of truth for test agents (T-11 stdlib-v1 gates
02.08). Derived from `plans/antipythany/master.md` (locked decisions, ecosystem tier
table), `plans/antipythany/00-foundation-contract/orchestrator.md` (00.03 compat
contract, 00.05 spec pack), `plans/antipythany/02-native-backend/orchestrator.md`
(02.08), and CPython **3.14.7** behavior verified on
`/opt/homebrew/bin/python3.14` (all name lists below were taken from live `-S` runs;
where a fact was checked, the check is cited inline).

Normative language: MUST / MUST NOT / SHOULD. Clause ids `S-05-NN` are citable by
corpus headers. This doc scopes **v1 only**; tier targets (T3/T4/T5) extend stdlib
only by PR against this file plus the compat contract.

## Amendment register

| A-id | Date | Clause | What changed | Evidence (live probe, CPython 3.14.7) |
| --- | --- | --- | --- | --- |
| A-6 | 2026-10-16 | S-05-4 (weakref row) | Pinned the measured weakref-TARGET matrix, which the row previously left open: `set`/`frozenset` ARE weakref-able on 3.14.7 (surprise vs the non-weakref-able mutable builtins); `list`, `dict`, `int`, `str`, `float`, `bytes`, `tuple`, plain `object()` instances are NOT (raise `TypeError: cannot create weak reference to 'X' object`); types and user class instances are. | `python3.14 -c "import weakref; weakref.ref(set())"` succeeds; same for `list()`/`dict()`/`1`/`'a'`/`1.0`/`b'a'`/`(1,)`/`object()` → TypeError |

---

## 5.1 Global rules

- **S-05-1** The v1 builtin namespace MUST equal the set of names in
  `dir(builtins)` of CPython 3.14 launched with `-S` (site disabled), minus the names
  explicitly marked EXCLUDED below. Verified baseline: `python3.14 -S -c "import
  builtins; print(sorted(dir(builtins)))"`. Under `-S`, `exit`, `quit`, `help`,
  `copyright`, `credits`, `license` are ABSENT (verified); under default startup they
  exist (site-injected). v1 is **-S-equivalent**: `print(exit)` MUST raise
  `NameError: name 'exit' is not defined` (verified under `-S`).
- **S-05-2** **Fail-loud rule** (compat contract 00.03): any `import` that resolves
  to a module OUT-of-v1 MUST produce a compile-time diagnostic naming the module and
  pointing to the extension policy doc. No stub module, no silent empty namespace,
  no runtime `ImportError` substitute. Applies to C-extension-backed modules and to
  pure-Python modules intentionally out of v1 scope alike.
- **S-05-3** A module is OUT-of-v1 if it imports a C extension with no pure-Python
  fallback, OR is marked OUT below by scope decision. CPython's own `_`-prefixed
  acceleration modules (`_functools`, `_collections`, `_sre`, `_operator`, `_bisect`,
  `_heapq`, `_json`, `_random`, `_csv`, `_socket`, ...) are never visible to user
  code; a pure-Python stdlib module that falls back when its `_x` accelerator is
  missing MAY be in v1 implemented natively.
- **S-05-4** Every IN-v1 module MUST expose at least the "v1 subset" enumerated in
  §5.3. Exposing additional stdlib names is allowed but each extra MUST cite this
  spec pack. Names NOT in the v1 subset MUST NOT be reachable (attribute access
  raises `AttributeError` matching CPython's message shape; import of a missing
  submodule obeys S-05-2).
- **S-05-5** Implementation route per 02.08: port from CPython's pure-Python stdlib
  source (PSF license — attribution in THIRD_PARTY_NOTICES) where possible, else
  native Rust citing clauses. Either route, observable behavior is gated by
  ap-conform byte-diff, so the ported/native distinction is invisible to the corpus.
- **S-05-6** `sys.stdlib_module_names` / `sys.builtin_module_names` content is NOT
  corpus-checked in v1 (they enumerate CPython's build; divergence is documented,
  not gated).

## 5.2 Builtins — exact v1 list

Grouped against the verified `python3.14 -S` `dir(builtins)`.

### 5.2.1 ALWAYS (core; no stdlib module dependency)

**S-05-7** Singleton constants: `None`, `True`, `False`, `Ellipsis`, `NotImplemented`
(incl. their dunder spellings `__debug__` runtime flag, `__doc__`, `__name__`,
`__package__`, `__spec__`, `__loader__` module attributes; `__build_class__` and
`__import__` are runtime machinery and MUST exist with CPython-compatible semantics).

**S-05-8** Core types: `bool`, `int`, `float`, `complex`, `str`, `bytes`,
`bytearray`, `list`, `dict`, `tuple`, `set`, `frozenset`, `object`, `type`,
`range`, `slice`, `property`, `classmethod`, `staticmethod`, `super`.

**S-05-9** Functions: `abs`, `all`, `any`, `ascii`, `bin`, `callable`, `chr`,
`delattr`, `divmod`, `enumerate`, `filter`, `format`, `getattr`, `hasattr`, `hash`,
`hex`, `id`, `isinstance`, `issubclass`, `iter`, `len`, `map`, `max`, `min`, `next`,
`oct`, `open`, `ord`, `pow`, `print`, `repr`, `reversed`, `round`, `setattr`,
`sorted`, `sum`, `vars`, `zip`.

**S-05-10** Exception hierarchy — all names present in `dir(builtins)` verified in
3.14.7: `ArithmeticError`, `AssertionError`, `AttributeError`, `BaseException`,
`BaseExceptionGroup`, `BlockingIOError`, `BrokenPipeError`, `BufferError`,
`BytesWarning`, `ChildProcessError`, `ConnectionAbortedError`, `ConnectionError`,
`ConnectionRefusedError`, `ConnectionResetError`, `DeprecationWarning`, `EOFError`,
`EncodingWarning`, `EnvironmentError`, `Exception`, `ExceptionGroup`,
`FileExistsError`, `FileNotFoundError`, `FloatingPointError`, `FutureWarning`,
`GeneratorExit`, `IOError`, `ImportError`, `ImportWarning`, `IndentationError`,
`IndexError`, `InterruptedError`, `IsADirectoryError`, `KeyError`,
`KeyboardInterrupt`, `LookupError`, `MemoryError`, `ModuleNotFoundError`,
`NameError`, `NotADirectoryError`, `NotImplementedError`, `OSError`,
`OverflowError`, `PendingDeprecationWarning`, `PermissionError`,
`ProcessLookupError`, `PythonFinalizationError`, `RecursionError`, `ReferenceError`,
`ResourceWarning`, `RuntimeError`, `RuntimeWarning`, `StopAsyncIteration`,
`StopIteration`, `SyntaxError`, `SyntaxWarning`, `SystemError`, `SystemExit`,
`TabError`, `TimeoutError`, `TypeError`, `UnicodeDecodeError`, `UnicodeEncodeError`,
`UnicodeError`, `UnicodeTranslateError`, `UnicodeWarning`, `UnboundLocalError`,
`UserWarning`, `ValueError`, `Warning`, `ZeroDivisionError`. Hierarchy shape (alias
`IOError is OSError`, `EnvironmentError is OSError`) per spec 02/03.

### 5.2.2 CONDITIONAL (available once the named dependency is in v1; otherwise fail-loud at first use per S-05-2 at the dependency boundary)

**S-05-11** `eval`, `exec`, `compile` — conditional on the runtime compile pipeline
(Plan 01/02). IN-v1: required by T2 (pytest assertion rewriting compiles AST).

**S-05-12** `globals`, `locals`, `dir` — conditional on frame/namespace introspection
(needed with `inspect`, §5.3). IN-v1.

**S-05-13** `aiter`, `anext` — conditional on async-iterator protocol. IN-v1 (T1
scrapy's deferreds-style async plus T4 asyncio later; the builtins themselves are
1-line protocol wrappers).

**S-05-14** `breakpoint` — namespace present, but v1 has no `pdb` (OUT-of-v1).
Calling it MUST raise `ModuleNotFoundError` naming `pdb` (CPython's default
`sys.breakpointhook` imports `pdb`). Corpus MUST NOT call it (interactive).

**S-05-15** `input` — conditional on `sys.stdin` readline-less passthrough; IN v1 as
blocking read from stdin. Corpus use is prohibited (ap-conform feeds no stdin;
EOFError only if stdin closed — mark corpus-prohibited, see spec 06).

### 5.2.3 EXCLUDED (names in CPython's `dir(builtins)` that v1 does NOT provide)

**S-05-16** `exit`, `quit`, `help`, `copyright`, `credits`, `license` — site-injected
only; v1 is -S-equivalent (S-05-1). Referencing them raises `NameError` — the same
byte-exact behavior as `python3.14 -S`.

**S-05-17** `memoryview` — deferred with the full buffer protocol (v2). Use of it
MUST fail loud at compile time if referenced (`NameError` is NOT acceptable for
`memoryview` because CPython-with-site exposes it: corpus anchors for `memoryview`
are prohibited in v1; this exclusion is a documented compat-contract divergence).

**S-05-18** `_IncompleteInputError` — readline/incomplete-input machinery, excluded.

## 5.3 Stdlib — v1 module list

IN = in v1 (native impl or PSF-source port). OUT = fail-loud import diagnostic
(S-05-2). "v1 subset" enumerates top-level names in scope; verified against
`python3.14 -S` live `dir(<module>)` output.

### 5.3.1 Core (named by 02.08 / compat contract)

| Module | Status | v1 subset (top-level names in scope) | Notes |
| --- | --- | --- | --- |
| `sys` | IN (runtime, not portable source) | `argv`, `path`, `modules`, `meta_path`, `path_hooks`, `path_importer_cache`, `stdout`, `stderr`, `stdin`, `platform`, `version`, `version_info`, `hexversion`, `api_version`, `maxsize`, `maxunicode`, `byteorder`, `prefix`, `exec_prefix`, `base_prefix`, `base_exec_prefix`, `builtin_module_names` (own registry), `flags`, `float_info`, `float_repr_style`, `int_info`, `hash_info`, `implementation`, `exc_info`, `excepthook`, `displayhook`, `breakpointhook`, `exit`, `getdefaultencoding`, `getfilesystemencoding`, `getfilesystemencodeerrors`, `intern`, `setrecursionlimit`, `getrecursionlimit`, `recursionlimit`, `getswitchinterval`, `setswitchinterval`, `getrefcount`/`getallocatedblocks` (may return stub-safe values, corpus-prohibited), `getsizeof` (corpus-prohibited), `orig_argv` | `sys.exit` semantics are spec 06. `sys.monitoring`, `sys._getframe` internals: `sys._getframe` IN (needed by `inspect.currentframe`); `addaudithook`/`audit` OUT; `remote_exec`, stack-trampoline APIs OUT. |
| `os` (os-lite) | IN | `sep`, `pathsep`, `curdir`, `pardir`, `extsep`, `altsep` (None on posix), `name`, `linesep`, `environ`, `environb`, `getenv`, `putenv`, `unsetenv`, `cwd` via `getcwd`, `getcwdb`, `listdir`, `scandir` (DirEntry), `stat`/`lstat`/`stat_result`/`statvfs`, `mkdir`, `makedirs`, `rmdir`, `removedirs`, `rename`, `replace`, `remove`, `unlink`, `walk`, `access`, `F_OK`/`R_OK`/`W_OK`/`X_OK`, `O_*` flags, `SEEK_SET`/`SEEK_CUR`/`SEEK_END`, `open`/`close`/`read`/`write`/`lseek`/`fdopen`, `chmod`, `umask`, `fsdecode`, `fsencode`, `fspath`, `PathLike`, `error` (=OSError), `getpid`, `getppid`, `strerror`, `devnull`, `TMP_MAX`, `cpu_count`/`process_cpu_count` | No `fork/exec/spawn/wait*`, no `pipe`, no `urandom`-as-corpus, no `register_at_fork`, no `kill`. Anything beyond the subset → S-05-2. |
| `os.path` | IN | `abspath`, `basename`, `dirname`, `isabs`, `join`, `normcase`, `normpath`, `split`, `splitext`, `splitdrive`, `commonprefix`, `commonpath`, `exists`, `lexists`, `isdir`, `isfile`, `islink`, `ismount`, `getsize`, `getatime`, `getmtime`, `getctime`, `realpath`, `relpath`, `samefile`, `sameopenfile`, `samestat`, `expanduser`, `expandvars`, constants (`sep` etc.), `ALLOW_MISSING` (3.14) | `posixpath`/`genericpath` are the porting sources. |
| `io` | IN | `IOBase`, `RawIOBase`, `BufferedIOBase`, `TextIOBase`, `BufferedReader`, `BufferedWriter`, `BufferedRWPair`, `BufferedRandom`, `TextIOWrapper`, `BytesIO`, `StringIO`, `FileIO`, `UnsupportedOperation`, `BlockingIOError`, `DEFAULT_BUFFER_SIZE`, `SEEK_*`, `open`, `IncrementalNewlineDecoder`, `text_encoding` | `Buffer` (3.14 new) SHOULD be present; `open_code` OUT. |
| `math` | IN (native Rust, CPython-exact) | all 62 names verified in 3.14.7 `dir(math)` (`acos`...`ulp`, incl. 3.14 additions `sumprod`, `fma`, `dist`-3+-arg, `isqrt`, `comb`, `perm`, `gcd`, `lcm`, `prod`, `nextafter`, `ulp`, `exp2`, `log2`, `cbrt`, `tau`, `inf`, `nan`, `pi`, `e`) | Byte-exact float results are gated only via corpus `repr` (spec 06); libm-platform divergence rule: corpus MUST pin values, not expressions, per compat contract. |
| `json` | IN | `dump`, `dumps`, `load`, `loads`, `JSONDecoder`, `JSONEncoder`, `JSONDecodeError`, `detect_encoding`, submodules `json.decoder`, `json.encoder`, `json.scanner` | pure-Python port is the baseline (C `_json` fallback exists in CPython too). `sort_keys`, separators, key order = insertion order. |
| `re` | IN | `compile`, `match`, `fullmatch`, `search`, `findall`, `finditer`, `sub`, `subn`, `split`, `purge`, `escape`, flags (`A`,`I`,`M`,`S`,`X`,`L`,`U`,`N`,`DOTALL`,`IGNORECASE`,`LOCALE`,`MULTILINE`,`NOFLAG`,`VERBOSE`,`ASCII`,`UNICODE`), `Pattern`/`Match` (+3.14 aliases `RegexFlag`, `PatternError`), `error` | `_sre` is invisible; `re.Scanner`/`copyreg`-hook re-import names OUT of corpus scope. `Match`/`Pattern` attributes per spec 01. 3.14 dual-interval engine: corpus anchors that print `.span()` MUST have identical results under both engines (avoid AT-splitting anchors). |
| `itertools` | IN | all 20 public names verified: `accumulate`, `batched`, `chain`, `combinations`, `combinations_with_replacement`, `compress`, `count`, `cycle`, `dropwhile`, `filterfalse`, `groupby`, `islice`, `pairwise`, `permutations`, `product`, `repeat`, `starmap`, `takewhile`, `tee`, `zip_longest` | |
| `functools` | IN | `cache`, `cached_property`, `cmp_to_key`, `get_cache_token`, `lru_cache`, `partial`, `partialmethod`, `recursive_repr`, `reduce`, `singledispatch`, `singledispatchmethod`, `total_ordering`, `update_wrapper`, `wraps`, `WRAPPER_ASSIGNMENTS`, `WRAPPER_UPDATES`, `Placeholder`, `RLock` | `namedtuple` re-export OUT-of-corpus (use `collections.namedtuple`). `_c3_merge` internals not visible. |
| `collections` | IN | `namedtuple`, `deque`, `defaultdict`, `OrderedDict`, `Counter`, `ChainMap`, `UserDict`, `UserList`, `UserString`, `abc` (submodule link) | `_collections` fallback verified pure-Python-capable in CPython; native impl must match `Counter.most_common` tie-break (insertion order) and `deque` block behavior only as observable. |
| `collections.abc` | IN | all 38 ABC names verified (`Awaitable`...`ValuesView`, `Buffer`, `Callable`, `Hashable`, `Iterable`, `Iterator`, `Mapping`, `MutableMapping`, `Sequence`, `MutableSequence`, `Set`, `MutableSet`, `Sized`, `Container`, `Collection`, `Reversible`, `Generator`, `AsyncGenerator`, `Coroutine`, `ByteString`, `KeysView`/`ItemsView`/`ValuesView`/`MappingView`, `ABCMeta`, `abstractmethod`) | Builtin iterator-type re-exports (`dict_keys`, `list_iterator`, ...) present by identity, corpus-prohibited as anchors except isinstance checks. |
| `types` | IN | `FunctionType`, `LambdaType`, `GeneratorType`, `CoroutineType`, `AsyncGeneratorType`, `BuiltinFunctionType`, `BuiltinMethodType`, `MethodType`, `MethodWrapperType`, `MethodDescriptorType`, `ClassMethodDescriptorType`, `GetSetDescriptorType`, `MemberDescriptorType`, `WrapperDescriptorType`, `CodeType`, `FrameType`, `TracebackType`, `ModuleType`, `CellType`, `SimpleNamespace`, `DynamicClassAttribute`, `MappingProxyType`, `NoneType`, `EllipsisType`, `NotImplementedType`, `UnionType`, `GenericAlias`, `new_class`, `prepare_class`, `resolve_bases`, `get_original_bases` | `CodeType` attribute surface needed by `inspect`; `CoroutineType` fields per async protocol. |

### 5.3.2 T1/T2 enablers (added per master tier table: scrapy generators/contextlib; pytest inspect/assertion-rewriting/import modes)

| Module | Status | v1 subset | Notes |
| --- | --- | --- | --- |
| `contextlib` | IN | `contextmanager`, `suppress`, `closing`, `asynccontextmanager`, `aclosing`, `nullcontext`, `ExitStack`, `AsyncExitStack`, `ContextDecorator`, `AsyncContextDecorator`, `redirect_stdout`, `redirect_stderr` | `chdir` IN but corpus-prohibited (mutates process state). |
| `inspect` (basics for T2) | IN | `signature`, `Signature`, `Parameter`, `BoundArguments`, `FullArgSpec`, `getfullargspec`, `getargs`, `getargvalues`, `Arguments`, `ArgInfo`, `currentframe`, `stack`, `getouterframes`, `getframeinfo`, `FrameInfo`, `Traceback` (class), `get_annotations`, `unwrap`, `isclass`, `isfunction`, `ismethod`, `ismodule`, `istraceback`, `isframe`, `iscode`, `isgenerator`, `isgeneratorfunction`, `isasyncgenfunction`, `iscoroutine`, `iscoroutinefunction`, `isawaitable`, `isbuiltin`, `isroutine`, `getmodule`, `getmodulename`, `getfile`, `getabsfile`, `getsourcefile`, `getsourcelines`, `getsource`, `findsource`, `getblock`, `cleandoc`, `getmembers`, `getmembers_static`, `getattr_static`, `classify_class_attrs`, `getmro`, `getclosurevars`, `ClosureVars`, `formatannotation`, `formatargvalues`, `Parameter kinds (POSITIONAL_ONLY...VAR_KEYWORD)` | Native-subset implementation (a port of inspect.py drags `dis`/`tokenize`/`ast`; each dep is IN or its use MUST be avoided). CO_* flags IN; `BlockFinder`, `TPFLAGS_IS_ABSTRACT`, generator-state funcs IN-lite; `markcoroutinefunction`/`FutureRef` OUT-of-corpus. |
| `linecache` | IN | `getline`, `getlines`, `updatecache`, `checkcache`, `clearcache`, `lazycache`, `cache` | traceback source lines depend on it (spec 06). |
| `token` | IN | `tok_name`, `EXACT_TOKEN_TYPES`, `IS*` predicates, `*` token constants | needed by `inspect`/`tokenize`; T2 assertion rewriting. |
| `tokenize` | IN | `generate_tokens`, `Token`, `TokenInfo`, `untokenize`, `detect_encoding`, `open`, `TokenError`, `tabsize`, `COMMENT`, `NL` (re-exported names subset) | needed by `inspect.getblock`/`findsource`. |
| `ast` | IN | `parse`, `unparse`, `walk`, `dump`, `literal_eval`, `NodeVisitor`, `NodeTransformer`, `fix_missing_locations`, `increment_lineno`, `get_docstring`, `get_source_segment`, `iter_child_nodes`, `iter_fields`, `copy_location`, full 3.14 node class set (as generated by the parser), `Module`, `Expression`, `Interactive`, `FunctionType`, `CToF flags (PyCF_*)` | pytest assertion rewriting is AST manipulation + `compile` (S-05-11); node `__repr__`/field order = CPython's. `main` OUT. |
| `enum` | IN | `Enum`, `IntEnum`, `StrEnum`, `Flag`, `IntFlag`, `ReprEnum`, `FlagBoundary`, `EnumCheck`, `auto`, `unique`, `member`, `nonmember`, `verify`, `show_flag_values`, `global_enum*` behaviors | 3.14 enum repr changes must be byte-matched via spec 01 repr rules. |
| `dataclasses` | IN | `dataclass`, `field`, `Field`, `fields`, `asdict`, `astuple`, `make_dataclass`, `replace`, `is_dataclass`, `InitVar`, `FrozenInstanceError`, `MISSING`, `KW_ONLY` | needs `typing.get_type_hints` for `__post_init__` string annotations — see `typing`. |
| `typing` | IN (introspection subset) | `Any`, `Optional`, `Union`, `Literal`, `ClassVar`, `Final`, `Annotated`, `TypeVar`, `Generic`, `Protocol`, `runtime_checkable`, `NamedTuple`, `TypedDict`, `Callable`, `Type`, `Self`, `Never`, `NoReturn`, `cast`, `overload`, `get_type_hints`, `get_origin`, `get_args`, `TYPE_CHECKING`, `ForwardRef`, `TypeAlias`, `TypeAliasType`, `NewType`, `ParamSpec`, `TypeVarTuple`, `Unpack`, `Required`, `NotRequired`, `TypeGuard`, `TypeIs`, `dataclass_transform`, `assert_never`, `assert_type`, `is_protocol`, `is_typeddict`, `get_protocol_members`, `reveal_type` (no-op print) | Legacy aliases (`List`, `Dict`, ...) IN as deprecated re-export names; `get_overloads`/`clear_overloads` IN-lite. Typing repr forms (`Union[int, str]` → `int | str`) MUST byte-match (spec 01 repr). |
| `string` | IN | `ascii_letters`, `ascii_lowercase`, `ascii_uppercase`, `digits`, `hexdigits`, `octdigits`, `printable`, `punctuation`, `whitespace`, `capwords`, `Formatter`, `Template` | |
| `textwrap` | IN | `TextWrapper`, `wrap`, `fill`, `shorten`, `indent`, `dedent` | |
| `pprint` | IN | `pprint`, `pformat`, `PrettyPrinter`, `saferepr`, `isreadable`, `isrecursive`, `pformat` | dict/order tie to spec 06 iteration-order rules. |
| `reprlib` | IN | `Repr`, `aRepr`, `recursive_repr` | |
| `operator` | IN | all ~73 names verified in 3.14.7 `dir(operator)` (`add`...`xor`, `attrgetter`, `itemgetter`, `methodcaller`, `length_hint`, `is_`, `is_not`, `truth`, `index`, `call`, ...) | `_operator`-fallback verified: star-import from `_operator` only when present — v1 implements natively. |
| `copy` | IN | `copy`, `deepcopy`, `Error`/`error`, `dispatch_table`, `replace` | `_keepalive` semantics: GC divergence documented; corpus MUST NOT assert copy+finalize ordering. |
| `numbers` | IN | `Number`, `Complex`, `Real`, `Rational`, `Integral` | |
| `abc` | IN | `ABC`, `ABCMeta`, `abstractmethod`, `abstractclassmethod`, `abstractstaticmethod`, `abstractproperty`, `get_cache_token`, `update_abstractmethods` | `_abc`/`_py_abc` dual fallback verified. |
| `atexit` | IN | `register`, `unregister` | LIFO order + runs on SystemExit path: verified (`bye` printed, `rc=3`; B then A LIFO). `sys.exitfunc` absent since 3.14 — N/A. |
| `warnings` | IN | `warn`, `warn_explicit`, `filterwarnings`, `simplefilter`, `resetwarnings`, `catch_warnings`, `formatwarning`, `showwarning`, `deprecated`, `filters`, `WarningMessage`, `defaultaction`, `onceregistry` | warning-line format `<file>:<lineno>: <Category>: <message>\n  <source>` byte-pinned in spec 06. |
| `weakref` | IN | `ref`, `proxy`, `ProxyType`, `CallableProxyType`, `ProxyTypes`, `ReferenceType`, `KeyedRef`, `WeakMethod`, `finalize`, `WeakKeyDictionary`, `WeakSet`, `WeakValueDictionary`, `getweakrefcount`/`getweakrefs` (corpus-prohibited results) | finalizer timing = documented divergence (compat contract; spec 06 §6.7). Weakref-TARGET matrix pinned live on 3.14.7 (Amendment A-6): `set` and `frozenset` ARE weakref-able; `list`, `dict`, `int`, `str`, `float`, `bytes`, `tuple`, and plain `object()` are NOT (`TypeError: cannot create weak reference to 'X' object`); types and user class instances are. |
| `traceback` | IN | `print_exc`, `format_exc`, `format_exception`, `format_exception_only`, `print_exception`, `format_tb`, `print_tb`, `format_list`, `print_list`, `format_stack`, `print_stack`, `extract_tb`, `extract_stack`, `walk_tb`, `walk_stack`, `TracebackException`, `StackSummary`, `FrameSummary`, `clear_frames`, `suppress` | MUST produce byte-identical text to the default excepthook rendering (spec 06 §6.3); `StackSummary.format_exception_only` includes 3.14's `^^^^` anchor rules. |
| `struct` | IN | `pack`, `unpack`, `pack_into`, `unpack_from`, `calcsize`, `iter_unpack`, `Struct`, `error` | pure-Python `struct.py` verified to re-export `_struct` (`from _struct import *`) → S-05-2 trap: v1 MUST implement natively; native-endianness formats (`@`, `=`) only, platform formats `!<>` per spec 01. |
| `heapq` | IN | `heappush`, `heappop`, `heapify`, `heapreplace`, `heappushpop`, `merge`, `nlargest`, `nsmallest`, `*_max` variants | verified `_heapq` fallback pattern. |
| `bisect` | IN | `bisect`, `bisect_left`, `bisect_right`, `insort`, `insort_left`, `insort_right` | verified `_bisect` fallback pattern. |
| `keyword` | IN | `iskeyword`, `issoftkeyword`, `kwlist`, `softkwlist` | needed by `dataclasses`/`ast` tooling. |
| `codecs` + `encodings` | IN (bootstrap subset) | `codecs`: `lookup`, `register`, `encode`, `decode`, `open`, `register_error`, `Codec`, `Incremental*` bases, BOM consts; `encodings`: registry + `aliases` | utf-8/ascii/latin-1/utf-16*/utf-32*/ascii/latin_1 mandatory; full Latin/Greek/CJK codec table OUT-of-corpus (documented). |
| `threading` | IN (02.07) | `Thread`, `current_thread`, `main_thread`, `active_count`, `enumerate`, `get_ident`, `get_native_id`, `Lock`, `RLock`, `Condition`, `Event`, `Semaphore`, `BoundedSemaphore`, `Barrier`, `BrokenBarrierError`, `Timer`, `local`, `TIMEOUT_MAX`, `ThreadError`, `excepthook`, `ExceptHookArgs` | corpus MAY assert logical ordering only (joined programs), never wall-clock timing (spec 06 §6.7). `getprofile`/`settrace`/`stack_size` OUT-of-corpus. |
| `time` | IN (minimal) | `monotonic`, `monotonic_ns`, `perf_counter`, `perf_counter_ns`, `sleep`, `time`, `time_ns`, `struct_time`, `gmtime`, `localtime`, `mktime`, `strftime`, `strptime`, `timezone`, `CLOCK_*` consts, `clock_gettime`, `get_clock_info` | value-bearing calls are corpus-prohibited (non-deterministic); presence + `sleep` semantics only. `_STRUCT_TM_ITEMS` internal. |
| `genericpath`, `posixpath` | IN (as `os.path` sources) | per `os.path` | `ntpath` compiled-but-unreachable on posix targets. |
| `stat` | IN | `S_*` consts, `ST_*` consts, `filemode`, `S_ISDIR`-style predicates | OS-agnostic constant values (posix). |
| `errno` | IN | `errorcode`, `E*` consts | const table per v1 target (darwin/linux values differ; corpus MUST use names, not raw numbers). |
| `argparse` (T2 enabler: pytest CLI) | IN | `ArgumentParser`, `Namespace`, `Action`, `ArgumentError`, `ArgumentTypeError`, `add_argument`/`parse_args`/`parse_known_args` full behavior, subparsers, help formatting | help text byte-matches CPython for same argv; terminal-size reflow = corpus-prohibited (depends on `shutil.get_terminal_size` env). |
| `platform` | IN (lite) | `system`, `machine`, `node`, `python_version`, `python_implementation`, `processor` (may return ''), `uname` | values on non-CPython = documented divergence; corpus MUST NOT anchor them. |
| `shutil` | OUT-of-v1 (scope) | — | needed by pytest for tmpdir cleanup? pytest degrades; tier-T2 claim may add a future slice by PR. |
| `pathlib` | IN (T2 enabler) | `Path`, `PurePosixPath`, `PureWindowsPath`, `PurePath`, `PosixPath`, `WindowsPath`, `Path` operator `/`, `exists`, `is_dir`, `is_file`, `iterdir`, `glob`, `rglob`, `read_text`, `read_bytes`, `write_text`, `write_bytes`, `mkdir`, `touch`, `unlink`, `joinpath`, `relative_to`, `resolve`, `absolute`, `as_posix`, `name`, `stem`, `suffix`, `parent`, `parts`, `with_name`, `with_suffix`, `samefile`, `stat`, `is_absolute`, `walk` (3.14), `__fspath__` | `PosixPath.cwd`/`home` IN. `PurePath` repr byte-matches. |
| `fnmatch` | IN | `fnmatch`, `fnmatchcase`, `filter`, `filterfalse`, `translate` | |
| `glob` | IN | `glob`, `iglob`, `escape`, `translate` | |
| `tempfile` | IN-lite (test-infra) | `TemporaryDirectory`, `NamedTemporaryFile`, `mkdtemp`, `gettempdir` | corpus-prohibited as anchor output (paths nondeterministic) but present for pytest fixtures. |
| `importlib` (basics) | IN | `import_module`, `reload` (documented limits), `importlib.util.find_spec`, `importlib.machinery` (`SourceFileLoader`, `BuiltinImporter`, `ExtensionFileLoader` → fail-loud per compat contract when an actual .so is found), `importlib.metadata` OUT | T2 "import modes". Spec 04 owns full import semantics; this table only scopes the names. |
| `unittest` | OUT-of-v1 | — | pytest (T2) does not require it; `unittest` corpus slices prohibited. |
| `pickle` | OUT-of-v1 (v1) | — | `_compat_pickle` chain + protocol versioning heavy; no T1/T2 gate. Fail-loud import. |

### 5.3.3 Explicit OUT (C-extension-backed or scope-excluded) — fail-loud per S-05-2

**S-05-19** The following are OUT-of-v1 (non-exhaustive, name-checked against
3.14.7): `_socket`/`socket`/`ssl`/`selectors`, `subprocess`, `multiprocessing`,
`ctypes`, `sqlite3`, `unicodedata`, `binascii`, `zlib`, `gzip`, `bz2`, `lzma`,
`zipfile`, `tarfile`, `hashlib`/`_hashlib`, `hmac`+`secrets` (depend on hashlib +
`os.urandom` beyond os-lite), `array`, `readline`, `curses`, `termios`, `fcntl`,
`pty`, `grp`, `pwd`, `syslog`, `mmap`, `resource`, `signal` (v1: no signal
handling — corpus prohibits `SIG*` use; the module imports the `_signal` builtin),
`random` (its `_random` MT19937 output is byte-observable and would require an
exact port; OUT-of-v1 scope decision, may become IN via PR), `statistics`
(depends on the `random`/`decimal` chain), `decimal` (`_decimal` C), `fractions`,
`datetime` (`_datetime` C; `time` subset above covers T1/T2 needs),
`tomllib` (`_tomllib` C), `pyexpat`/`xml`, `http`, `urllib`, `email`, `asyncio`
(v1: generators IN, full event loop lands with the T4 plan; OUT until then — T1
scrapy's core spider run avoids it), `logging` (OUT-of-v1 scope: pytest does not
require it; may become IN by PR with T3), `concurrent.futures`, `contextvars`
(`_contextvars` C — async-context depth; OUT until T4), `zoneinfo`, `dbm`,
`_ossaudiodev`, `tkinter`, `cProfile`/`profile`/`pstats`, `pdb`/`bdb`/`doctest`,
`shlex`/`getopt` (pure Python but no v1 target imports them; OUT-of-corpus),
`csv` (`_csv` accelerator fallback exists but dialect byte-exactness of the pure
path is unverified → OUT until an anchor slice proves portability), `difflib`
(pure; pytest's assertion diffing does NOT use it → OUT-of-v1 scope),
`unittest` (see table), `site` (v1 is -S-equivalent per S-05-1; importing `site`
itself fail-loud), `sysconfig`, `runpy` (OUT until CLI `-m` lands in Plan 04; 04
verbs then add by PR), `pickle` (see table).

**S-05-20** Also OUT-of-v1 by scope decision (even where pure-Python is possible):
`base64`, `uuid`, `ipaddress`, `netrc`, `configparser`, `html`, `calendar`,
`cmath` (C extension in 3.14.7 — not in `sys.builtin_module_names`, loads from
lib-dynload), `dis`, `symtable`, `compileall`, `pkgutil`, `modulefinder`, `shlex`,
`getopt`, `optparse`, `stringprep`, `unicodedata` (repeated from §5.3.3 for
emphasis). `dis`/`symtable` are pure Python but bound to CPython bytecode formats
that v1's IR does not replicate — they MUST NOT be ported; imports fail loud
(S-05-2).

**S-05-21** `marshal`, `faulthandler`, `_thread`, `_imp`, `posix` are
runtime-internal modules with no user-visible import contract in v1 (imports fail
loud per S-05-2). `builtins` IS importable (IN: its whole `dir()` is the S-05-7..10
namespace). `gc` MUST NOT be importable from corpus code (its counters are
inherently impl-specific, prohibited by spec 06 §6.7).

## 5.4 Module-presence test protocol

**S-05-22** For every IN module, corpus slice `stdlib-v1` MUST include at least one
import-smoke case and one behavior case per v1-subset row group. For every OUT
module listed in §5.3.3, corpus MUST include a fail-loud case: `import <mod>` →
compile-time diagnostic (native mode) / equivalent refusal (wasm mode); the
diagnostic text is NOT byte-gated against CPython (CPython succeeds) — it is gated
by the compat-contract diagnostic format (names the module + policy pointer).

**S-05-23** Conditional builtins (§5.2.2) that depend on v1 features MUST exist in
`dir(builtins)` in v1; calling with a missing dependency raises at runtime exactly
as CPython-with-the-same-subset would (e.g. `breakpoint()` → `ModuleNotFoundError:
No module named 'pdb'` — verified shape for missing-module import under 3.14.7:
`ModuleNotFoundError: No module named 'nonexist_mod'`).

---

## Corpus anchors (verified on CPython 3.14.7, `python3.14 -S`)

Each anchor: one-line script → exact stdout → exit code. All outputs below were
produced by running the snippet on `/opt/homebrew/bin/python3.14` (3.14.7).

1. **S-05-1/S-05-16** `print(exit)` → stdout ``; stderr
   `Traceback (most recent call last):\n  File "<string>", line 1, in <module>\n    print(exit)\n          ^^^^\nNameError: name 'exit' is not defined` ; exit 1 (site names absent, -S parity).
2. **S-05-9/S-05-10** `print(abs(-3), sorted([3,1,2]), isinstance(True, int), issubclass(bool, int), divmod(17,5), pow(2,10), len({1,2}))` → `3 [1, 2, 3] True True (3, 2) 1024 2` ; exit 0. (`map`/`filter` render verified too: `print(list(map(str, range(3))), list(filter(None, [0,1,'',2])))` → `['0', '1', '2'] [1, 2]`.)
3. **S-05-11** `compile`/`eval` path: `eval('1+')` → last stderr lines
   `  File "<string>", line 1\n    1+\nSyntaxError: invalid syntax` (under a
   `<string>` traceback frame) ; exit 1.
4. **math IN** `import math; print(math.isqrt(10**20), math.gcd(12,18), math.pi, math.floor(2.7))` → `10000000000 6 3.141592653589793 2` ; exit 0.
5. **json IN** `import json; print(json.dumps({'b':1,'a':[1,2.5,True,None]}, sort_keys=True))` → `{"a": [1, 2.5, true, null], "b": 1}` ; exit 0. (`json.loads('{"x": [1, 2.0]}')` → `{'x': [1, 2.0]}` ; exit 0.)
6. **re IN** `import re; print(re.findall(r'\d+', 'a1b22'), re.sub(r'\s+', '-', 'x  y'))` → `['1', '22'] x-y` ; exit 0.
7. **itertools IN** `import itertools; print(list(itertools.islice(itertools.count(2), 3)), list(itertools.chain([1],[2])), list(itertools.batched('abc', 2)))` → `[2, 3, 4] [1, 2] [('a', 'b'), ('c',)]` ; exit 0.
8. **functools+collections IN** `import functools, collections; print(functools.reduce(lambda a,b: a+b, [1,2,3]), collections.Counter('aab')['a'])` → `6 2` ; exit 0.
9. **contextlib IN (T1)** `with contextlib.suppress(ZeroDivisionError): 1/0` then `print('ok')` → `ok` ; exit 0.
10. **inspect basics IN (T2)** `def f(a, b=2, *args, **kw): pass` → `print(inspect.signature(f), inspect.isfunction(f))` → `(a, b=2, *args, **kw) True` ; exit 0. (`getfullargspec(f)` → `FullArgSpec(args=['a', 'b'], varargs='args', varkw='kw', defaults=(2,), kwonlyargs=[], kwonlydefaults=None, annotations={})`.)
11. **dataclasses/enum IN** `@dataclasses.dataclass class P: x: int` → `print(P(1))` → `P(x=1)` ; exit 0; and `class C(enum.IntEnum): A = 1` → `print(C.A, int(C.A)+1, C.A == 1)` → `1 2 True` ; exit 0.
12. **S-05-2 OUT fail-loud** `import socket` → compile-time diagnostic naming `socket` (native/wasm) ; NOT byte-gated vs CPython (per S-05-22).


---

## Amendment A1 (2026-10-17, orchestrator + user-approved protocol)

**S-05-19 modified:** `base64`, `binascii`, `hashlib`/`_hashlib`-surface
(md5/sha1/sha224/sha256/sha384/sha512), `random` are IN-v1. Rationale: the
OUT classification assumed C-extension backing or entropy-surface risk;
02.08-lite delivered them as native Rust (RustCrypto for digests, exact
MT19937 for random), byte-exact vs 3.14.7 with 19 differential probes
(crates/ap-run/tests/stdlib_modules_differential.rs). `secrets` remains OUT
(os.urandom dependency). `binascii` is IN as the base64 dependency surface.
Clause ids S-05-19/S-05-20 are NOT renumbered; this amendment narrows their
name lists. Contract-change rule: PR-based amendment recorded here per
C-32 (ids never reused, history preserved in git).
