"""T-11 stdlib-v1 scope table — transcription of docs/spec/05-builtins-stdlib-v1.md.

This module is DATA, not semantics: every entry below is transcribed verbatim
from spec 05 (the S-05-NN rows of §5.2/§5.3) with the citing clause kept next to
it. The test suite (test_t11_stdlib.py) asserts (a) internal consistency of the
table, (b) that the table's IN-subset names exist on the reference CPython
3.14.7 (`/opt/homebrew/bin/python3.14 -S`, where the row claims -S verification),
and (c) — xfail until leaf 02.08 lands — that the antipythony module registry
matches the table: IN modules import, OUT modules fail loud with the C-10 token
for their class, and PARTIAL rows keep out-of-scope names unreachable (S-05-4).

Authorship: written from the spec pack ONLY (spec 05 + compat contract C-04..C-10
+ leaf 02.08 Expose/Gate). No crates/*/src was read.

Transcription rules used (filing deviations as GAPS, never silently re-scoping):
- "v1 subset" columns are copied name-for-name; where spec 05's text and the
  live 3.14.7 -S `dir()` disagree, the spec text is transcribed into the table
  and the name is ALSO listed in KNOWN_REF_ABSENT so the reference-vs-table
  check stays honest about which mismatches are known (see GAP list in
  plans/antipythony/tests/T-11-stdlib-v1.md).
- Module-level status is IN / PARTIAL / OUT. Spec 05 literally labels rows
  "IN", "IN (…)", "IN-lite", "OUT-of-v1"; PARTIAL here = any IN row whose
  Notes/column names explicit out-of-scope names (subset-bounded rows). The
  PARTIAL boundary reading and its ambiguities are filed as CONTRACT-GAPs.
- Fail-loud token classes (C-10): C-extension-backed OUT modules expect
  APX-CEXT (C-07/C-08); pure-Python scope-excluded OUT modules expect
  APX-NOV1 (C-06); names that do not exist at all expect APX-NOTFOUND.
"""

# ---------------------------------------------------------------------------
# Builtins (S-05-7 .. S-05-18)
# ---------------------------------------------------------------------------

# S-05-16: site-injected names, v1 is -S-equivalent; referencing them raises
# NameError byte-exact vs `python3.14 -S`.
BUILTINS_EXCLUDED_SITE = ["exit", "quit", "help", "copyright", "credits", "license"]

# S-05-17: deferred with the buffer protocol (v2). Use MUST fail loud at
# compile time; a corpus anchor is prohibited; documented divergence.
BUILTINS_EXCLUDED_OTHER = ["memoryview"]  # S-05-17
# S-05-18: readline/incomplete-input machinery, excluded.
BUILTINS_EXCLUDED_INTERNAL = ["_IncompleteInputError"]  # S-05-18

# S-05-7..10 + §5.2.2 conditional builtins that ARE in v1:
BUILTINS_IN = {
    # S-05-7 singletons/dunders
    "singletons": ["None", "True", "False", "Ellipsis", "NotImplemented",
                   "__debug__", "__doc__", "__name__", "__package__",
                   "__spec__", "__loader__", "__build_class__", "__import__"],
    # S-05-8 core types
    "types": ["bool", "int", "float", "complex", "str", "bytes", "bytearray",
              "list", "dict", "tuple", "set", "frozenset", "object", "type",
              "range", "slice", "property", "classmethod", "staticmethod",
              "super"],
    # S-05-9 functions
    "functions": ["abs", "all", "any", "ascii", "bin", "callable", "chr",
                  "delattr", "divmod", "enumerate", "filter", "format",
                  "getattr", "hasattr", "hash", "hex", "id", "isinstance",
                  "issubclass", "iter", "len", "map", "max", "min", "next",
                  "oct", "open", "ord", "pow", "print", "repr", "reversed",
                  "round", "setattr", "sorted", "sum", "vars", "zip"],
    # S-05-10 exceptions (subset probed by T-11; full list in spec 05 §5.2.1)
    "exceptions_probed": ["ArithmeticError", "AttributeError", "Exception",
                          "ImportError", "ModuleNotFoundError", "NameError",
                          "OSError", "ValueError", "TypeError", "KeyError",
                          "ZeroDivisionError", "OverflowError", "IndexError",
                          "RuntimeError", "StopIteration", "SystemExit",
                          "FloatingPointError"],
    # S-05-11 (eval/exec/compile IN — Plan 01/02 pipeline), S-05-12, S-05-13:
    "conditional_in": ["eval", "exec", "compile", "globals", "locals", "dir",
                       "aiter", "anext"],
    # S-05-14: breakpoint namespace-present, calling raises ModuleNotFoundError
    # naming 'pdb'. S-05-15: input IN, corpus use prohibited.
    "namespace_only_or_prohibited": ["breakpoint", "input"],
}

# ---------------------------------------------------------------------------
# Stdlib modules — IN (full rows of spec 05 §5.3.1/§5.3.2)
# Each value: {"cite": clause/row citation, "subset": v1-subset names verbatim,
#              "out": names the row explicitly excludes (PARTIAL probes)}
# ---------------------------------------------------------------------------

IN_MODULES = {
    # ---- §5.3.1 Core ----
    "sys": {
        "cite": "S-05-4 (spec 05 §5.3.1 row `sys`; S-05-6, S-05-21)",
        "subset": ["argv", "path", "modules", "meta_path", "path_hooks",
                   "path_importer_cache", "stdout", "stderr", "stdin",
                   "platform", "version", "version_info", "hexversion",
                   "api_version", "maxsize", "maxunicode", "byteorder",
                   "prefix", "exec_prefix", "base_prefix", "base_exec_prefix",
                   "builtin_module_names", "flags", "float_info",
                   "float_repr_style", "int_info", "hash_info",
                   "implementation", "exc_info", "excepthook", "displayhook",
                   "breakpointhook", "exit", "getdefaultencoding",
                   "getfilesystemencoding", "getfilesystemencodeerrors",
                   "intern", "setrecursionlimit", "getrecursionlimit",
                   "recursionlimit", "getswitchinterval", "setswitchinterval",
                   "getrefcount", "getallocatedblocks", "getsizeof",
                   "orig_argv"],
        "ref_absent": ["recursionlimit"],  # GAP-3 (attr absent; getrecursionlimit present)
        "subset_notes": "_getframe IN per Notes but absent from the subset column (GAP-4); "
                        "getrefcount/getallocatedblocks/getsizeof corpus-prohibited (C-19).",
        "out": ["addaudithook", "audit", "monitoring"],  # row Notes: OUT
    },
    "os": {
        "cite": "S-05-4 (spec 05 §5.3.1 row `os` (os-lite); C-05 floor)",
        "subset": ["sep", "pathsep", "curdir", "pardir", "extsep", "altsep",
                   "name", "linesep", "environ", "environb", "getenv",
                   "putenv", "unsetenv", "getcwd", "getcwdb", "listdir",
                   "scandir", "stat", "lstat", "stat_result", "statvfs",
                   "mkdir", "makedirs", "rmdir", "removedirs", "rename",
                   "replace", "remove", "unlink", "walk", "access", "F_OK",
                   "R_OK", "W_OK", "X_OK", "SEEK_SET", "SEEK_CUR", "SEEK_END",
                   "open", "close", "read", "write", "lseek", "fdopen",
                   "chmod", "umask", "fsdecode", "fsencode", "fspath",
                   "PathLike", "error", "getpid", "getppid", "strerror",
                   "devnull", "TMP_MAX", "cpu_count", "process_cpu_count"],
        "subset_notes": "row writes \"cwd via getcwd\" — transcribed as getcwd "
                        "(no os.cwd attribute on the reference).",
        "out": ["fork", "execv", "spawnv", "wait", "waitpid", "pipe", "kill",
                "register_at_fork", "urandom"],  # row Notes "No fork/exec/spawn/wait*, no pipe, ... no kill"
    },
    "os.path": {
        "cite": "S-05-4 (spec 05 §5.3.1 row `os.path`)",
        "subset": ["abspath", "basename", "dirname", "isabs", "join",
                   "normcase", "normpath", "split", "splitext", "splitdrive",
                   "commonprefix", "commonpath", "exists", "lexists", "isdir",
                   "isfile", "islink", "ismount", "getsize", "getatime",
                   "getmtime", "getctime", "realpath", "relpath", "samefile",
                   "sameopenfile", "samestat", "expanduser", "expandvars",
                   "ALLOW_MISSING"],
        "out": [],
    },
    "genericpath": {
        "cite": "S-05-4 (spec 05 §5.3.2 row `genericpath, posixpath` — os.path "
                "porting sources; GAP-17: bare `import genericpath` under -S "
                "fails a CPython circular-import edge (verified live), so no "
                "-S dir() claim is transcribed for it)",
        "subset": [],
        "out": [],
    },
    "io": {
        "cite": "S-05-4 (spec 05 §5.3.1 row `io`)",
        "subset": ["IOBase", "RawIOBase", "BufferedIOBase", "TextIOBase",
                   "BufferedReader", "BufferedWriter", "BufferedRWPair",
                   "BufferedRandom", "TextIOWrapper", "BytesIO", "StringIO",
                   "FileIO", "UnsupportedOperation", "BlockingIOError",
                   "DEFAULT_BUFFER_SIZE", "open", "IncrementalNewlineDecoder",
                   "text_encoding", "SEEK_SET", "SEEK_CUR", "SEEK_END"],
        "out": ["open_code"],  # row Notes: `open_code` OUT
        "should": ["Buffer"],  # row Notes: 3.14-new `Buffer` SHOULD be present (GAP-2: absent on -S reference)
    },
    "math": {
        "cite": "S-05-4 (spec 05 §5.3.1 row `math`; all 62 names verified 3.14.7)",
        "subset": ["acos", "acosh", "asin", "asinh", "atan", "atan2", "atanh",
                   "cbrt", "ceil", "comb", "copysign", "cos", "cosh",
                   "degrees", "dist", "e", "erf", "erfc", "exp", "exp2",
                   "expm1", "fabs", "factorial", "floor", "fma", "fmod",
                   "frexp", "fsum", "gamma", "gcd", "hypot", "inf", "isclose",
                   "isfinite", "isinf", "isnan", "isqrt", "lcm", "ldexp",
                   "lgamma", "log", "log10", "log1p", "log2", "modf", "nan",
                   "nextafter", "perm", "pi", "pow", "prod", "radians",
                   "remainder", "sin", "sinh", "sqrt", "sumprod", "tan",
                   "tanh", "tau", "trunc", "ulp"],
        "out": ["__file__"],  # acceleration-module identity probe (S-05-3: native, no file)
    },
    "json": {
        "cite": "S-05-4 (spec 05 §5.3.1 row `json`)",
        "subset": ["dump", "dumps", "load", "loads", "JSONDecoder",
                   "JSONEncoder", "JSONDecodeError", "detect_encoding",
                   "decoder", "encoder", "scanner"],
        "out": [],
    },
    "re": {
        "cite": "S-05-4 (spec 05 §5.3.1 row `re`)",
        "subset": ["compile", "match", "fullmatch", "search", "findall",
                   "finditer", "sub", "subn", "split", "purge", "escape",
                   "A", "I", "M", "S", "X", "L", "U", "N", "DOTALL",
                   "IGNORECASE", "LOCALE", "MULTILINE", "NOFLAG", "VERBOSE",
                   "ASCII", "UNICODE", "Pattern", "Match", "RegexFlag",
                   "PatternError", "error"],
        "ref_absent": ["N"],  # row lists flag `N` but 3.14.7 -S hasattr(re,'N') is False (GAP-5)
        "out": [],
        "corpus_out": ["Scanner"],  # row Notes: OUT of corpus scope (not registry-out) — GAP
    },
    "itertools": {
        "cite": "S-05-4 (spec 05 §5.3.1 row `itertools`; all 20 names verified)",
        "subset": ["accumulate", "batched", "chain", "combinations",
                   "combinations_with_replacement", "compress", "count",
                   "cycle", "dropwhile", "filterfalse", "groupby", "islice",
                   "pairwise", "permutations", "product", "repeat", "starmap",
                   "takewhile", "tee", "zip_longest"],
        "out": [],
    },
    "functools": {
        "cite": "S-05-4 (spec 05 §5.3.1 row `functools`)",
        "subset": ["cache", "cached_property", "cmp_to_key", "get_cache_token",
                   "lru_cache", "partial", "partialmethod", "recursive_repr",
                   "reduce", "singledispatch", "singledispatchmethod",
                   "total_ordering", "update_wrapper", "wraps",
                   "WRAPPER_ASSIGNMENTS", "WRAPPER_UPDATES", "Placeholder",
                   "RLock"],
        "out": [],
        "corpus_out": ["namedtuple"],  # row Notes: re-export OUT-of-corpus
    },
    "collections": {
        "cite": "S-05-4 (spec 05 §5.3.1 row `collections`)",
        "subset": ["namedtuple", "deque", "defaultdict", "OrderedDict",
                   "Counter", "ChainMap", "UserDict", "UserList",
                   "UserString", "abc"],
        "out": [],
    },
    "collections.abc": {
        "cite": "S-05-4 (spec 05 §5.3.1 row `collections.abc`; \"all 38 ABC names\" — GAP-10)",
        # Unique names enumerated by the row (26 public __all__ names on the
        # 3.14.7 -S reference; GAP-10 files the "38" count vs the row's own list).
        "subset": ["Awaitable", "Callable", "Collection", "Container",
                   "Coroutine", "Generator", "Hashable", "Iterable",
                   "Iterator", "Reversible", "AsyncGenerator",
                   "AsyncIterable", "AsyncIterator",
                   "ByteString", "KeysView", "ItemsView", "ValuesView",
                   "Mapping", "MappingView", "MutableMapping",
                   "MutableSequence", "MutableSet", "Sequence", "Set",
                   "Sized", "ABCMeta", "abstractmethod"],
        "ref_absent": [],
        "out": ["Buffer"],  # 3.14.7 -S dir(collections.abc) has no Buffer; row text names it (GAP-11)
    },
    "types": {
        "cite": "S-05-4 (spec 05 §5.3.1 row `types`)",
        "subset": ["FunctionType", "LambdaType", "GeneratorType",
                   "CoroutineType", "AsyncGeneratorType", "BuiltinFunctionType",
                   "BuiltinMethodType", "MethodType", "MethodWrapperType",
                   "MethodDescriptorType", "ClassMethodDescriptorType",
                   "GetSetDescriptorType", "MemberDescriptorType",
                   "WrapperDescriptorType", "CodeType", "FrameType",
                   "TracebackType", "ModuleType", "CellType",
                   "SimpleNamespace", "DynamicClassAttribute",
                   "MappingProxyType", "NoneType", "EllipsisType",
                   "NotImplementedType", "UnionType", "GenericAlias",
                   "new_class", "prepare_class", "resolve_bases",
                   "get_original_bases"],
        "out": [],
    },
    "builtins": {
        "cite": "S-05-21 (importable; whole dir() is the S-05-7..10 namespace)",
        "subset": [],  # namespace governed by BUILTINS_IN
        "out": [],
    },
    # ---- §5.3.2 T1/T2 enablers ----
    "contextlib": {
        "cite": "S-05-4 (spec 05 §5.3.2 row `contextlib`)",
        "subset": ["contextmanager", "suppress", "closing",
                   "asynccontextmanager", "aclosing", "nullcontext",
                   "ExitStack", "AsyncExitStack", "ContextDecorator",
                   "AsyncContextDecorator", "redirect_stdout",
                   "redirect_stderr"],
        "out": [],
        "corpus_out": ["chdir"],  # IN but corpus-prohibited (mutates process state)
    },
    "inspect": {
        "cite": "S-05-4 (spec 05 §5.3.2 row `inspect`)",
        "subset": ["signature", "Signature", "Parameter", "BoundArguments",
                   "FullArgSpec", "getfullargspec", "getargs", "getargvalues",
                   "Arguments", "ArgInfo", "currentframe", "stack",
                   "getouterframes", "getframeinfo", "FrameInfo", "Traceback",
                   "get_annotations", "unwrap", "isclass", "isfunction",
                   "ismethod", "ismodule", "istraceback", "isframe", "iscode",
                   "isgenerator", "isgeneratorfunction", "isasyncgenfunction",
                   "iscoroutine", "iscoroutinefunction", "isawaitable",
                   "isbuiltin", "isroutine", "getmodule", "getmodulename",
                   "getfile", "getabsfile", "getsourcefile", "getsourcelines",
                   "getsource", "findsource", "getblock", "cleandoc",
                   "getmembers", "getmembers_static", "getattr_static",
                   "classify_class_attrs", "getmro", "getclosurevars",
                   "ClosureVars", "formatannotation", "formatargvalues"],
        "ref_absent": [],
        "out": ["FutureRef"],  # Notes: `markcoroutinefunction`/`FutureRef` OUT-of-corpus; FutureRef absent on reference too (GAP-12)
        "corpus_out": ["markcoroutinefunction"],
    },
    "linecache": {
        "cite": "S-05-4 (spec 05 §5.3.2 row `linecache`)",
        "subset": ["getline", "getlines", "updatecache", "checkcache",
                   "clearcache", "lazycache", "cache"],
        "out": [],
    },
    "token": {
        "cite": "S-05-4 (spec 05 §5.3.2 row `token`)",
        "subset": ["tok_name", "EXACT_TOKEN_TYPES", "ISEOF", "ISNONTERMINAL",
                   "ISTERMINAL"],
        "out": [],
    },
    "tokenize": {
        "cite": "S-05-4 (spec 05 §5.3.2 row `tokenize`)",
        "subset": ["generate_tokens", "Token", "TokenInfo", "untokenize",
                   "detect_encoding", "open", "TokenError", "tabsize",
                   "COMMENT", "NL"],
        "out": [],
    },
    "ast": {
        "cite": "S-05-4 (spec 05 §5.3.2 row `ast`)",
        "subset": ["parse", "unparse", "walk", "dump", "literal_eval",
                   "NodeVisitor", "NodeTransformer", "fix_missing_locations",
                   "increment_lineno", "get_docstring", "get_source_segment",
                   "iter_child_nodes", "iter_fields", "copy_location",
                   "Module", "Expression", "Interactive", "FunctionType",
                   "PyCF_ONLY_AST"],
        "out": ["main"],  # Notes: `main` OUT
    },
    "enum": {
        "cite": "S-05-4 (spec 05 §5.3.2 row `enum`)",
        "subset": ["Enum", "IntEnum", "StrEnum", "Flag", "IntFlag", "ReprEnum",
                   "FlagBoundary", "EnumCheck", "auto", "unique", "member",
                   "nonmember", "verify", "show_flag_values"],
        "out": [],
    },
    "dataclasses": {
        "cite": "S-05-4 (spec 05 §5.3.2 row `dataclasses`)",
        "subset": ["dataclass", "field", "Field", "fields", "asdict",
                   "astuple", "make_dataclass", "replace", "is_dataclass",
                   "InitVar", "FrozenInstanceError", "MISSING", "KW_ONLY"],
        "out": [],
    },
    "typing": {
        "cite": "S-05-4 (spec 05 §5.3.2 row `typing` (introspection subset))",
        "subset": ["Any", "Optional", "Union", "Literal", "ClassVar", "Final",
                   "Annotated", "TypeVar", "Generic", "Protocol",
                   "runtime_checkable", "NamedTuple", "TypedDict", "Callable",
                   "Type", "Self", "Never", "NoReturn", "cast", "overload",
                   "get_type_hints", "get_origin", "get_args", "TYPE_CHECKING",
                   "ForwardRef", "TypeAlias", "TypeAliasType", "NewType",
                   "ParamSpec", "TypeVarTuple", "Unpack", "Required",
                   "NotRequired", "TypeGuard", "TypeIs", "dataclass_transform",
                   "assert_never", "assert_type", "is_protocol",
                   "is_typeddict", "get_protocol_members", "reveal_type"],
        "out": [],
        "extra_in": ["get_overloads", "clear_overloads"],  # Notes: IN-lite
        "legacy_in": ["List", "Dict"],  # Notes: deprecated re-export names IN
    },
    "string": {
        "cite": "S-05-4 (spec 05 §5.3.2 row `string`)",
        "subset": ["ascii_letters", "ascii_lowercase", "ascii_uppercase",
                   "digits", "hexdigits", "octdigits", "printable",
                   "punctuation", "whitespace", "capwords", "Formatter",
                   "Template"],
        "out": [],
    },
    "textwrap": {
        "cite": "S-05-4 (spec 05 §5.3.2 row `textwrap`)",
        "subset": ["TextWrapper", "wrap", "fill", "shorten", "indent",
                   "dedent"],
        "out": [],
    },
    "pprint": {
        "cite": "S-05-4 (spec 05 §5.3.2 row `pprint`)",
        "subset": ["pprint", "pformat", "PrettyPrinter", "saferepr",
                   "isreadable", "isrecursive"],
        "out": [],
    },
    "reprlib": {
        "cite": "S-05-4 (spec 05 §5.3.2 row `reprlib`)",
        "subset": ["Repr", "aRepr", "recursive_repr"],
        "out": [],
    },
    "operator": {
        "cite": "S-05-4 (spec 05 §5.3.2 row `operator`; \"~73 names\" vs 57 -S public — GAP-13)",
        "subset": ["add", "and_", "attrgetter", "concat", "contains", "countOf",
                   "delitem", "eq", "floordiv", "ge", "getitem", "gt",
                   "iadd", "iand", "iconcat", "ilshift", "imatmul",
                   "imod", "imul", "index", "inv", "ior", "ipow", "irshift",
                   "is_", "is_not", "isub", "itemgetter", "itruediv", "ixor",
                   "le", "length_hint", "lshift", "lt", "matmul", "methodcaller",
                   "mod", "mul", "ne", "neg", "not_", "or_", "pos", "pow",
                   "rshift", "setitem", "sub", "truth", "truediv", "xor",
                   "call"],
        "ref_absent": ["call"],  # spec example name; absent on 3.14.7 (GAP-13)
        "out": [],
    },
    "copy": {
        "cite": "S-05-4 (spec 05 §5.3.2 row `copy`)",
        "subset": ["copy", "deepcopy", "Error", "error", "dispatch_table",
                   "replace"],
        "out": [],
    },
    "numbers": {
        "cite": "S-05-4 (spec 05 §5.3.2 row `numbers`)",
        "subset": ["Number", "Complex", "Real", "Rational", "Integral"],
        "out": [],
    },
    "abc": {
        "cite": "S-05-4 (spec 05 §5.3.2 row `abc`)",
        "subset": ["ABC", "ABCMeta", "abstractmethod", "abstractclassmethod",
                   "abstractstaticmethod", "abstractproperty",
                   "get_cache_token", "update_abstractmethods"],
        "out": [],
    },
    "atexit": {
        "cite": "S-05-4 (spec 05 §5.3.2 row `atexit`)",
        "subset": ["register", "unregister"],
        "out": [],
    },
    "warnings": {
        "cite": "S-05-4 (spec 05 §5.3.2 row `warnings`)",
        "subset": ["warn", "warn_explicit", "filterwarnings", "simplefilter",
                   "resetwarnings", "catch_warnings", "formatwarning",
                   "showwarning", "deprecated", "filters", "WarningMessage",
                   "defaultaction", "onceregistry"],
        "out": [],
    },
    "weakref": {
        "cite": "S-05-4 (spec 05 §5.3.2 row `weakref`; finalizer timing = D1/C-22)",
        "subset": ["ref", "proxy", "ProxyType", "CallableProxyType",
                   "ProxyTypes", "ReferenceType", "KeyedRef", "WeakMethod",
                   "finalize", "WeakKeyDictionary", "WeakSet",
                   "WeakValueDictionary", "getweakrefcount", "getweakrefs"],
        "out": [],
    },
    "traceback": {
        "cite": "S-05-4 (spec 05 §5.3.2 row `traceback`)",
        "subset": ["print_exc", "format_exc", "format_exception",
                   "format_exception_only", "print_exception", "format_tb",
                   "print_tb", "format_list", "print_list", "format_stack",
                   "print_stack", "extract_tb", "extract_stack", "walk_tb",
                   "walk_stack", "TracebackException", "StackSummary",
                   "FrameSummary", "clear_frames", "suppress"],
        "out": [],
    },
    "struct": {
        "cite": "S-05-4 (spec 05 §5.3.2 row `struct`; S-05-2 trap: `_struct` re-export ⇒ native impl)",
        "subset": ["pack", "unpack", "pack_into", "unpack_from", "calcsize",
                   "iter_unpack", "Struct", "error"],
        "out": [],
    },
    "heapq": {
        "cite": "S-05-4 (spec 05 §5.3.2 row `heapq`)",
        "subset": ["heappush", "heappop", "heapify", "heapreplace",
                   "heappushpop", "merge", "nlargest", "nsmallest",
                   "heapify_max", "heappop_max", "heappush_max",
                   "heappushpop_max", "heapreplace_max"],
        "out": [],
    },
    "bisect": {
        "cite": "S-05-4 (spec 05 §5.3.2 row `bisect`)",
        "subset": ["bisect", "bisect_left", "bisect_right", "insort",
                   "insort_left", "insort_right"],
        "out": [],
    },
    "keyword": {
        "cite": "S-05-4 (spec 05 §5.3.2 row `keyword`)",
        "subset": ["iskeyword", "issoftkeyword", "kwlist", "softkwlist"],
        "out": [],
    },
    "codecs": {
        "cite": "S-05-4 (spec 05 §5.3.2 row `codecs` + `encodings` (bootstrap subset))",
        "subset": ["lookup", "register", "encode", "decode", "open",
                   "register_error", "Codec", "BOM", "BOM_BE", "BOM_LE",
                   "BOM_UTF8", "BOM_UTF16",
                   "BOM_UTF16_BE", "BOM_UTF16_LE", "BOM_UTF32",
                   "BOM_UTF32_BE", "BOM_UTF32_LE",
                   "IncrementalEncoder", "IncrementalDecoder",
                   "BufferedIncrementalEncoder", "BufferedIncrementalDecoder",
                   "StreamReader", "StreamWriter", "StreamReaderWriter",
                   "StreamRecoder"],
        "out": [],
        "corpus_out_note": "full Latin/Greek/CJK codec tables OUT-of-corpus (row Notes) — GAP-14",
    },
    "encodings": {
        "cite": "S-05-4 (spec 05 §5.3.2 row `codecs` + `encodings`)",
        "subset": ["aliases"],
        "out": [],
    },
    "threading": {
        "cite": "S-05-4 (spec 05 §5.3.2 row `threading` (02.07); logical ordering only per spec 06 §6.7)",
        "subset": ["Thread", "current_thread", "main_thread", "active_count",
                   "enumerate", "get_ident", "get_native_id", "Lock", "RLock",
                   "Condition", "Event", "Semaphore", "BoundedSemaphore",
                   "Barrier", "BrokenBarrierError", "Timer", "local",
                   "TIMEOUT_MAX", "ThreadError", "excepthook",
                   "ExceptHookArgs"],
        "out": [],
        "corpus_out": ["getprofile", "settrace", "stack_size"],  # OUT-of-corpus (GAP-15: corpus-only vs registry)
    },
    "time": {
        "cite": "S-05-4 (spec 05 §5.3.2 row `time` (minimal)); value-bearing calls corpus-prohibited",
        "subset": ["monotonic", "monotonic_ns", "perf_counter",
                   "perf_counter_ns", "sleep", "time", "time_ns",
                   "struct_time", "gmtime", "localtime", "mktime", "strftime",
                   "strptime", "timezone", "clock_gettime", "get_clock_info",
                   "CLOCK_MONOTONIC", "CLOCK_REALTIME"],
        "out": [],
        # strictest S-05-4 reading: names below the subset are unreachable on
        # a PARTIAL row — reference HAS them; filed as GAP-16 in the leaf doc.
        "partial_out_of_subset": ["asctime", "ctime", "tzset", "tzname",
                                  "daylight", "altzone", "process_time",
                                  "thread_time"],
    },
    "posixpath": {
        "cite": "S-05-4 (spec 05 §5.3.2 row `genericpath, posixpath` — porting sources; GAP-17: bare `-S` import of genericpath fails circular-import)",
        "subset": ["join", "split", "dirname", "basename", "exists", "isdir"],
        "out": [],
    },
    "stat": {
        "cite": "S-05-4 (spec 05 §5.3.2 row `stat`)",
        "subset": ["S_IFDIR", "S_IFREG", "S_IFLNK", "S_ISDIR", "S_ISREG",
                   "S_ISLNK", "filemode", "ST_SIZE", "ST_MODE", "ST_MTIME"],
        "out": [],
    },
    "errno": {
        "cite": "S-05-4 (spec 05 §5.3.2 row `errno`; corpus MUST use names not raw numbers)",
        "subset": ["errorcode", "ENOENT", "EEXIST", "EACCES", "EPERM",
                   "EINTR", "EINVAL"],
        "out": [],
    },
    "argparse": {
        "cite": "S-05-4 (spec 05 §5.3.2 row `argparse` (T2 enabler)); terminal-size reflow corpus-prohibited",
        "subset": ["ArgumentParser", "Namespace", "Action", "ArgumentError",
                   "ArgumentTypeError"],
        "out": [],
    },
    "platform": {
        "cite": "S-05-4 (spec 05 §5.3.2 row `platform` (lite)); values are documented divergence — presence only",
        "subset": ["system", "machine", "node", "python_version",
                   "python_implementation", "processor", "uname"],
        "out": [],
    },
    "pathlib": {
        "cite": "S-05-4 (spec 05 §5.3.2 row `pathlib` (T2 enabler))",
        # The row's subset column lists Path INSTANCE methods/dunders; at
        # module level the row's names are the six path classes. The
        # instance-method names are verified corpus-side (s044), not via
        # module hasattr.
        "subset": ["Path", "PurePosixPath", "PureWindowsPath", "PurePath",
                   "PosixPath", "WindowsPath"],
        "instance_subset": ["exists", "is_dir", "is_file", "iterdir", "glob",
                            "rglob", "read_text", "read_bytes", "write_text",
                            "write_bytes", "mkdir", "touch", "unlink",
                            "joinpath", "relative_to", "resolve", "absolute",
                            "as_posix", "name", "stem", "suffix", "parent",
                            "parts", "with_name", "with_suffix", "samefile",
                            "stat", "is_absolute", "walk", "__fspath__"],
        "out": [],
    },
    "fnmatch": {
        "cite": "S-05-4 (spec 05 §5.3.2 row `fnmatch`)",
        "subset": ["fnmatch", "fnmatchcase", "filter", "filterfalse",
                   "translate"],
        "ref_absent": ["filterfalse"],  # 3.14.7 dir(fnmatch) lacks it — GAP-1
        "out": [],
    },
    "glob": {
        "cite": "S-05-4 (spec 05 §5.3.2 row `glob`)",
        "subset": ["glob", "iglob", "escape", "translate"],
        "out": [],
    },
    "tempfile": {
        "cite": "S-05-4 (spec 05 §5.3.2 row `tempfile` (IN-lite (test-infra)); anchor output prohibited)",
        "subset": ["TemporaryDirectory", "NamedTemporaryFile", "mkdtemp",
                   "gettempdir"],
        "out": ["mkstemp", "SpooledTemporaryFile"],  # strictest PARTIAL reading (GAP-18)
    },
    "importlib": {
        "cite": "S-05-4 (spec 05 §5.3.2 row `importlib` (basics); spec 04 owns full import semantics)",
        "subset": ["import_module", "reload"],
        "out": ["metadata"],  # row: importlib.metadata OUT ⇒ submodule import fail-loud
    },
}

# ---------------------------------------------------------------------------
# PARTIAL rows — IN modules whose spec-05 row explicitly puts named
# objects out of the v1 subset. Reading used (strictest per S-05-4): a name
# the row marks OUT is UNREACHABLE on antipythony (AttributeError per
# S-05-4 first sentence; submodule import per S-05-2/C-10 token per second),
# even though the CPython reference still has it — so the row's expected
# bytes DIFFER from the reference (GAP-26/GAP-27 in the leaf doc).
# Fields: module, kind ("attr" or "submodule"), name, disposition
# ("registry-out" → fail-loud APX token + no attribute; "keep-present" →
# registry keeps the name, only corpus anchoring is prohibited → GAP-28).
# ---------------------------------------------------------------------------

PARTIAL_ROWS = [
    {"mod": "sys", "kind": "attr", "name": "addaudithook",
     "disposition": "registry-out",
     "cite": "S-05-4 §5.3.1 row sys Notes: `addaudithook`/`audit` OUT"},
    {"mod": "sys", "kind": "attr", "name": "monitoring",
     "disposition": "registry-out",
     "cite": "S-05-4 §5.3.1 row sys Notes (GAP-29: `sys.monitoring` OUT vs "
             "C-05 native-provided internals wording)"},
    {"mod": "sys", "kind": "attr", "name": "audit",
     "disposition": "registry-out",
     "cite": "S-05-4 §5.3.1 row sys Notes: `addaudithook`/`audit` OUT"},
    {"mod": "os", "kind": "attr", "name": "fork",
     "disposition": "registry-out",
     "cite": "S-05-4 §5.3.1 row os Notes: \"No fork/exec/spawn/wait*\" → S-05-2"},
    {"mod": "os", "kind": "attr", "name": "kill",
     "disposition": "registry-out",
     "cite": "S-05-4 §5.3.1 row os Notes: \"no kill\" → S-05-2"},
    {"mod": "os", "kind": "attr", "name": "pipe",
     "disposition": "registry-out",
     "cite": "S-05-4 §5.3.1 row os Notes: \"no pipe\" → S-05-2"},
    {"mod": "os", "kind": "attr", "name": "register_at_fork",
     "disposition": "registry-out",
     "cite": "S-05-4 §5.3.1 row os Notes → S-05-2"},
    {"mod": "io", "kind": "attr", "name": "open_code",
     "disposition": "registry-out",
     "cite": "S-05-4 §5.3.1 row io Notes: `open_code` OUT (GAP-26: S-05-4 "
             "names-not-reachable vs ap-conform byte-exactness)"},
    {"mod": "re", "kind": "attr", "name": "Scanner",
     "disposition": "keep-present",
     "cite": "S-05-4 §5.3.1 row re Notes: `re.Scanner` OUT of CORPUS scope "
             "(not registry) — corpus-prohibition reading (GAP-28)"},
    {"mod": "functools", "kind": "attr", "name": "namedtuple",
     "disposition": "keep-present",
     "cite": "S-05-4 §5.3.1 row functools Notes: re-export OUT-of-corpus — "
             "corpus-prohibition reading (GAP-28)"},
    {"mod": "threading", "kind": "attr", "name": "getprofile",
     "disposition": "keep-present",
     "cite": "S-05-4 §5.3.2 row threading Notes: OUT-of-corpus (GAP-15/GAP-28)"},
    {"mod": "contextlib", "kind": "attr", "name": "chdir",
     "disposition": "keep-present",
     "cite": "S-05-4 §5.3.2 row contextlib Notes: \"chdir IN but "
             "corpus-prohibited\" — unambiguous: keep in registry (GAP-18 analogue)"},
    {"mod": "ast", "kind": "attr", "name": "main",
     "disposition": "registry-out",
     "cite": "S-05-4 §5.3.2 row ast Notes: `main` OUT — same Notes column as "
             "rows whose OUTs are corpus-only, read as registry-out (GAP-26)"},
    {"mod": "inspect", "kind": "attr", "name": "markcoroutinefunction",
     "disposition": "keep-present",
     "cite": "S-05-4 §5.3.2 row inspect Notes: OUT-of-corpus (GAP-28)"},
    {"mod": "inspect", "kind": "attr", "name": "FutureRef",
     "disposition": "registry-out",
     "cite": "S-05-4 §5.3.2 row inspect Notes: `markcoroutinefunction`/"
             "`FutureRef` OUT-of-corpus; FutureRef absent on reference too — "
             "transcribed registry-out (GAP-12)"},
    {"mod": "time", "kind": "attr", "name": "asctime",
     "disposition": "registry-out",
     "cite": "S-05-4 (row time minimal): name outside the subset MUST NOT be "
             "reachable — strictest reading (GAP-16)"},
    {"mod": "tempfile", "kind": "attr", "name": "mkstemp",
     "disposition": "registry-out",
     "cite": "S-05-4 (row tempfile IN-lite subset): strictest reading (GAP-18)"},
    {"mod": "os", "kind": "attr", "name": "urandom",
     "disposition": "keep-present",
     "cite": "S-05-4 §5.3.1 row os Notes: \"no urandom-as-corpus\" — "
             "corpus-prohibition reading (GAP-18); OUT-of-v1 `random` closure "
             "consequence recorded as GAP-19"},
    {"mod": "importlib", "kind": "submodule", "name": "metadata",
     "disposition": "registry-out",
     "cite": "S-05-4 §5.3.2 row importlib: `importlib.metadata` OUT → import "
             "obey S-05-2 (APX-NOV1/C-06), NOT ModuleNotFoundError-with-"
             "APX-NOV1 at runtime (task wording vs S-05-2 'no runtime "
             "ImportError substitute' — resolved to compile-time token)"},
]

# ---------------------------------------------------------------------------
# Stdlib modules — OUT (§5.3.2 table rows + S-05-19/20/21). Class drives the
# expected C-10 token: "cext" → APX-CEXT (extension-backed, C-07/C-08),
# "nov1" → APX-NOV1 (pure-Python but scope-excluded, C-06).
# ---------------------------------------------------------------------------

OUT_MODULES = {
    # §5.3.2 table rows explicitly OUT-of-v1
    "shutil": ("nov1", "S-05-4 §5.3.2 row `shutil` = OUT-of-v1 (scope)"),
    "unittest": ("nov1", "S-05-4 §5.3.2 row `unittest` = OUT-of-v1; S-05-19"),
    "pickle": ("cext", "S-05-4 §5.3.2 row `pickle` = OUT (v1); `_pickle` C, S-05-19"),
    # S-05-19 — C-extension-backed
    "socket": ("cext", "S-05-19 (`_socket`)"),
    "ssl": ("cext", "S-05-19"),
    "subprocess": ("cext", "S-05-19"),
    "multiprocessing": ("cext", "S-05-19"),
    "ctypes": ("cext", "S-05-19"),
    "sqlite3": ("cext", "S-05-19"),
    "unicodedata": ("cext", "S-05-19"),
    "binascii": ("cext", "S-05-19"),
    "zlib": ("cext", "S-05-19"),
    "gzip": ("cext", "S-05-19"),
    "bz2": ("cext", "S-05-19"),
    "lzma": ("cext", "S-05-19"),
    "zipfile": ("cext", "S-05-19 (closure hits binascii/zlib)"),
    "hashlib": ("cext", "S-05-19 (`_hashlib`)"),
    "array": ("cext", "S-05-19"),
    "readline": ("cext", "S-05-19"),
    "mmap": ("cext", "S-05-19"),
    "resource": ("cext", "S-05-19"),
    "datetime": ("cext", "S-05-19 (`_datetime`)"),
    "decimal": ("cext", "S-05-19 (`_decimal`)"),
    "tomllib": ("cext", "S-05-19 (`_tomllib`)"),
    "xml": ("cext", "S-05-19 (`pyexpat`)"),
    "http": ("cext", "S-05-19"),
    "urllib": ("cext", "S-05-19"),
    "email": ("cext", "S-05-19"),
    "contextvars": ("cext", "S-05-19 (`_contextvars`)"),
    "selectors": ("nov1", "S-05-19 (pure-Python; socket-dead closure ⇒ see GAP-19)"),
    # S-05-19 scope decisions on pure-Python
    "logging": ("nov1", "S-05-19 OUT-of-v1 scope"),
    "csv": ("nov1", "S-05-19: OUT until an anchor slice proves portability"),
    "difflib": ("nov1", "S-05-19 OUT-of-v1 scope (pure)"),
    "statistics": ("nov1", "S-05-19 (depends on random/decimal chain; closure hits C — GAP-19)"),
    "asyncio": ("nov1", "S-05-19 (OUT until the T4 plan; closure hits _contextvars — GAP-19)"),
    "site": ("nov1", "S-05-19: importing `site` itself fail-loud (S-05-1)"),
    "sysconfig": ("nov1", "S-05-19"),
    "runpy": ("nov1", "S-05-19 (OUT until CLI -m lands in Plan 04)"),
    "pdb": ("nov1", "S-05-19 (`pdb`/`bdb`/`doctest`)"),
    "doctest": ("nov1", "S-05-19"),
    "shlex": ("nov1", "S-05-19/S-05-20: pure but no v1 target imports them"),
    "signal": ("nov1", "S-05-19 (v1: no signal handling; GAP-20: row says module imports `_signal` builtin)"),
    # S-05-20 — scope-excluded although pure-Python-capable
    "base64": ("nov1", "S-05-20"),
    "uuid": ("nov1", "S-05-20 (closure hits binascii — GAP-19)"),
    "ipaddress": ("nov1", "S-05-20"),
    "configparser": ("nov1", "S-05-20"),
    "html": ("nov1", "S-05-20"),
    "calendar": ("nov1", "S-05-20"),
    "cmath": ("cext", "S-05-20 (C extension in 3.14.7, loads from lib-dynload)"),
    "dis": ("nov1", "S-05-20: MUST NOT be ported; imports fail loud"),
    "symtable": ("nov1", "S-05-20: MUST NOT be ported"),
    "pkgutil": ("nov1", "S-05-20"),
    "compileall": ("nov1", "S-05-20"),
    "getopt": ("nov1", "S-05-20"),
    "optparse": ("nov1", "S-05-20"),
    "stringprep": ("nov1", "S-05-20"),
    # S-05-21 — runtime-internal, no user-visible import contract
    "marshal": ("nov1", "S-05-21"),
    "faulthandler": ("cext", "S-05-21"),
    "_imp": ("nov1", "S-05-21"),
    "posix": ("nov1", "S-05-21"),
    "gc": ("nov1", "S-05-21: MUST NOT be importable from corpus code (GAP-21: corpus-prohibition vs registry-absence is ambiguous)"),
    # accelerator modules never visible to user code (S-05-3) — C-07 probe set
    "_socket": ("cext", "S-05-3/S-05-19; C-07 Verify probe set"),
    "_ssl": ("cext", "C-07 Verify probe set"),
    "_json": ("cext", "S-05-3; C-07 Verify probe set (json must not expose it)"),
    "_sqlite3": ("cext", "S-05-19 (`sqlite3`); C-05 Verify probe set"),
    "_hashlib": ("cext", "S-05-19"),
    "_random": ("cext", "S-05-19 (`random` row)"),
    "_sre": ("nov1", "S-05-3: invisible accelerator"),
    "_functools": ("nov1", "S-05-3: invisible accelerator"),
    "_collections": ("nov1", "S-05-3: invisible accelerator"),
    "_operator": ("nov1", "S-05-3: invisible accelerator"),
    "_bisect": ("nov1", "S-05-3: invisible accelerator"),
    "_heapq": ("nov1", "S-05-3: invisible accelerator"),
    "_csv": ("nov1", "S-05-3: invisible accelerator"),
    "_struct": ("nov1", "S-05-3/S-05-2 trap on the struct row"),
    "_decimal": ("cext", "S-05-19"),
    "_datetime": ("cext", "S-05-19"),
    "_thread": ("nov1", "S-05-21 (runtime-internal; GAP-22: C-05 names it native-provided in closures — user-facing import still fails loud)"),
}

# `random` is a CONTRACT CONFLICT, deliberately excluded from both lists:
# C-05's "named in-v1 floor" includes `random` (removal needs a PR against
# compat-contract.md), while spec 05 S-05-19 marks it OUT-of-v1. The contract
# wins on conflict (compat-contract.md §0/§11), so T-11 does not assert
# random's fail-loud status; GAP-23 in the leaf doc files the contradiction.
CONFLICT_MODULES = ["random"]

# Names in spec-05 subset columns that the live 3.14.7 -S reference does NOT
# have (transcription kept; reference-consistency test whitelists exactly these).
KNOWN_REF_ABSENT = {
    "sys": ["recursionlimit"],           # GAP-3 (attr absent; getrecursionlimit present)
    "re": ["N"],                          # GAP-5
    "fnmatch": ["filterfalse"],           # GAP-1
}
