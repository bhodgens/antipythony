# T-11 — stdlib-v1 scope + behavior (gates implementation leaf 02.08 v1 stdlib subset)

Authorship: written from the spec pack ONLY (`docs/spec/05-builtins-stdlib-v1.md`,
`docs/spec/06-observable-behavior.md`, `docs/compat-contract.md`,
`plans/antipythany/master.md`, `plans/antipythany/tests/README.md`, and leaf 02.08's
Expose/Gate in `plans/antipythany/02-native-backend/orchestrator.md`). The author has
not read any implementation source under `crates/`.

## Purpose

Gate leaf 02.08 ("v1 stdlib subset"): the exact module list from spec 05, ported
from CPython pure-Python source or implemented natively, exposed as a module
registry. Three faces:

1. **Allow-list face** — a static-assertion suite over `tests/t11_stdlib/scope_table.py`,
   a clause-cited transcription of spec 05 §5.2/§5.3 (IN/PARTIAL/OUT). It checks the
   table's internal consistency, its honesty against the live CPython 3.14.7 `-S`
   reference (every subset name must exist on the reference unless whitelisted as a
   filed transcription gap), the divergence premise (all OUT modules import fine on
   the reference — CPython never refuses them), and — once 02.08 lands — that the
   antipythany registry matches the table: IN modules import and smoke, OUT modules
   refuse the build with the C-10 token for their class (C-05/C-06/C-07/C-08/C-10).
2. **Behavior face** — the differential corpus `tests/corpus/stdlib/` (48 cases), one
   or more per IN module row, every byte pinned by real `/opt/homebrew/bin/python3.14`
   runs: import-smoke + behavior per S-05-22, with the spec-05 anchors #1–#11 values
   embedded, edge error texts (math ValueError/OverflowError, exact JSONDecodeError
   format, re error texts), observable lru_cache stats, Counter/deque/OrderedDict/
   namedtuple repr+ordering, dataclass field order/repr/eq/frozen error, contextlib
   suppress/closing/contextmanager/ExitStack LIFO, typing runtime checks, sys/os-lite
   attribute presence. Determinism enforced: plain-vs-`-S` runs byte-agree (site
   independence, S-05-1), unordered projections sorted, no salted hashes, no
   refcount/`id()`-value/`input()`/`memoryview`/`breakpoint()`/`urandom` anchors
   (C-19/D3, S-05-14/15/17), no wall-clock values (row `time`), no raw errno numbers
   (row `errno`), no platform/path/env values (rows `platform`/`tempfile`/`warnings`).
3. **PARTIAL face** — for each IN row whose Notes put named objects outside the v1
   subset (`PARTIAL_ROWS` in the scope table), one xfail test asserting the
   documented-in-scope names exist and the out-of-scope names are unreachable
   (AttributeError shape) or fail loud as submodule imports (APX-NOV1 per
   S-05-2/C-06) exactly as read below.

## Testability contract under test (from leaf 02.08's Expose/Gate)

- Expose: module registry (the exact spec-05 list).
- Gate (verbatim): "ap-conform stdlib slice green in tier-0 AND adaptive AND binary
  modes."
- The subprocess seam is ap-conform as documented by 00.04: `ap-conform run
  --corpus <glob> --impl <cmd>`, three channels (S-06-1/C-27), separate streams
  (S-06-2). Resolution order copied from T-06: env `AP_CONFORM` → PATH →
  `target/{debug,release}/`; impl via env `APX_IMPL` else `antipythany run`; reference
  via `AP_CONFORM_REFERENCE_PYTHON` default `/opt/homebrew/bin/python3.14`. Binary
  mode has no ap-conform seam in 00.04's CLI, so the third gate leg builds each
  sampled case (`antipythany build`) and diffs the artifact's three channels against
  the pinned triple directly.
- OUT-module refusals are NOT byte-gated against CPython (S-05-22: "the diagnostic
  text is NOT byte-gated against CPython (CPython succeeds)"); they are gated against
  the compat-contract diagnostic format: token + module name + `file:line:col`
  (C-08/C-10 Verify), and no artifact (C-11).
- Fail-loud is asserted at the BUILD seam (compile-time per S-05-2/C-07), never as a
  runtime `ImportError`/`ModuleNotFoundError` substitute — see GAP-27.

## Scope-table transcription note

`tests/t11_stdlib/scope_table.py` is data transcribed from spec 05's IN/PARTIAL/OUT
tables with a clause citation on every row. Rules (deviations filed as gaps, never
silently re-scoped):

- Subset columns copied name-for-name, including names the live 3.14.7 `-S`
  reference does NOT have (`sys.recursionlimit`, `re.N`, `fnmatch.filterfalse`,
  `operator.call/get`-style example names) — spec 05's preamble claims those columns
  were "verified against python3.14 -S live dir()", so each mismatch is a
  spec-vs-reference contradiction listed in `KNOWN_REF_ABSENT`/`ref_absent` and
  probed (with the whitelist as the assertion's tolerance) by
  `test_in_subset_names_exist_on_reference`.
- Status mapping: `IN` = row marked IN (any qualifier). `OUT` = §5.3.2 table rows
  marked OUT-of-v1 + §5.3.3 (S-05-19), §5.3.4 (S-05-20), S-05-21, plus the S-05-3
  invisible accelerator modules `_json`/`_sre`/... as user imports. Token class:
  `cext`→APX-CEXT, `nov1`→APX-NOV1 (C-06 reason distinction; see GAP-19 for
  closure-derived rows where the class is genuinely ambiguous).
- `PARTIAL` = an IN row whose Notes/subset column explicitly excludes named objects.
  Two dispositions: `registry-out` (names written plainly "OUT" — S-05-4's
  names-MUST-NOT-be-reachable sentence applies) and `keep-present` (names written
  "OUT-of-corpus"/"corpus-prohibited" — prohibited as corpus anchors only; GAP-28).
- `random` sits in NEITHER list: C-05's named floor says IN (removal requires a
  contract PR), S-05-19 says OUT. T-11 asserts the conflict is on the books rather
  than resolving it (GAP-23).
- Builtins: `BUILTINS_EXCLUDED_SITE` (S-05-16 NameError parity), `memoryview`
  (S-05-17 compile-time fail-loud, corpus-prohibited anchor), `_IncompleteInputError`
  (S-05-18). Site names absent under `-S` on the reference — verified (corpus anchor
  #1 shape pinned via the -S lint, not a crashing case).

## Case table (tests/corpus/stdlib/, all rc-0 unless noted; triples expected/<stem>.{out,err,rc})

| Case | Module(s) / face | Cites |
| --- | --- | --- |
| s001_builtins_namespace | builtins namespace, conditional builtins, eval/compile error shape | S-05-7..13; anchors #2/#3 |
| s002_sys_core_attrs | sys presence + deterministic projections | S-05-4 (row sys), S-05-6, C-16 |
| s003_os_lite | os-lite constants/environ/fs booleans | S-05-4 (row os), C-05 |
| s004_ospath_string_ops | os.path pure-string ops + absent-path predicates | S-05-4 (row os.path) |
| s005_io_streams | io classes, BytesIO/StringIO/TextIOWrapper ops | S-05-4 (row io), S-06-19 |
| s006_math_basics | math subset presence + pinned values | S-05-4 (row math), anchor #4, C-15 |
| s007_math_edges_errors | math ValueError/OverflowError text pins | S-05-4, C-14, S-06-7 |
| s008_json_roundtrip | dumps/loads/sort_keys/indent/separators/cls | S-05-4 (row json), anchor #5, C-26 |
| s009_json_errors | exact JSONDecodeError message + attrs | S-05-4, C-14/C-25 |
| s010_re_match_subset | compile/groups/sub/split/finditer/flags | S-05-4 (row re), anchor #6 |
| s011_re_errors | re.error message pins | S-05-4, C-14 |
| s012_itertools_subset | all 20 itertools names + outputs | S-05-4 (row itertools), anchor #7 |
| s013_functools_reduce_partial | reduce/partial/wraps/partialmethod/singledispatch/total_ordering/cached_property/recursive_repr | S-05-4 (row functools), anchor #8 |
| s014_functools_lru_stats | lru_cache cache_info/cache_parameters/cache_clear | S-05-4 (row functools) |
| s015_collections_types | Counter/deque/defaultdict/OrderedDict/namedtuple/ChainMap reprs+ordering | S-05-4 (row collections), anchor #8, C-26 |
| s016_collections_abc_isinstance | 27 ABC names + isinstance graph + subclass hooks | S-05-4 (row collections.abc) |
| s017_types_identities | types identities incl. NoneType/UnionType/new_class | S-05-4 (row types) |
| s018_contextlib_suppress_closing | suppress/closing/contextmanager/redirect | S-05-4 (row contextlib), anchor #9, T1 |
| s019_contextlib_exitstack | ExitStack LIFO + enter_context unwind | S-05-4 (row contextlib) |
| s020_inspect_basics | signature/getfullargspec/predicates/cleandoc | S-05-4 (row inspect), anchor #10, T2 |
| s021_linecache | linecache ops incl. real-file line 1 | S-05-4 (row linecache) |
| s022_token_tokenize | token tables + generate_tokens/untokenize | S-05-4 (rows token, tokenize) |
| s023_ast_parse_dump_unparse | parse/dump/unparse/literal_eval/visitors | S-05-4 (row ast), S-05-11 |
| s024_enum_subset | Enum/IntEnum/StrEnum/Flag/IntFlag reprs + unique/verify | S-05-4 (row enum), anchor #11 |
| s025_dataclasses_fields | field order/repr/eq/frozen error/KW_ONLY/InitVar/replace | S-05-4 (row dataclasses), anchor #11 |
| s026_typing_runtime | Union repr byte form/get_origin/runtime_checkable/NamedTuple/TypedDict | S-05-4 (row typing), S-01 repr rules |
| s027_string_textwrap | string consts/Template errors/Formatter + textwrap | S-05-4 (rows string, textwrap) |
| s028_pprint_reprlib | pformat order (C-26 stable), aRepr truncation | S-05-4 (rows pprint, reprlib) |
| s029_operator_subset | operator ~50-name subset + getters | S-05-4 (row operator), S-05-3 |
| s030_copy_module | copy/deepcopy cycles/replace, no finalize asserts | S-05-4 (row copy), C-19 |
| s031_numbers_abc | numbers ABC tower + abc error text + update_abstractmethods | S-05-4 (rows numbers, abc) |
| s032_atexit_lifo | atexit LIFO + unregister + SystemExit path | S-05-4 (row atexit), C-28/C-29, S-06-11 | rc 3 pinned |
| s033_warnings_format | formatwarning byte shape, catch_warnings(record) | S-05-4 (row warnings), S-06-10 |
| s034_weakref_basics | ref/proxy/Weak*/finalize.detach (no timing asserts) | S-05-4 (row weakref), C-22/C-23 |
| s035_traceback_format_only | format_exception_only + walk_tb/TracebackException shape | S-05-4 (row traceback), C-25 |
| s036_struct_heapq_bisect | struct packs + error texts, heapq incl. *_max, bisect | S-05-4 (rows struct, heapq, bisect), S-05-2 trap |
| s037_keyword_codecs_encodings | keyword + codecs bootstrap (BOMs, incremental, aliases) | S-05-4 (rows keyword, codecs+encodings) |
| s038_threading_smoke | presence + joined logical order only | S-05-4 (row threading), spec 06 §6.7 |
| s039_time_minimal | presence + gmtime(0)/strftime determinism only | S-05-4 (row time) |
| s040_stat_errno | filemode pins, errno via NAMES not numbers | S-05-4 (rows stat, errno) |
| s041_posixpath_sources | posixpath identity with os.path | S-05-4 (row genericpath/posixpath), GAP-17 |
| s042_argparse_subset | fixed-argv parse/parse_known_args/subparsers/exit codes | S-05-4 (row argparse), T2 |
| s043_platform_lite | presence + arity/type only (values = divergence) | S-05-4 (row platform) |
| s044_pathlib_pure | PurePath repr/ops byte pins, absent-path booleans | S-05-4 (row pathlib), T2 |
| s045_fnmatch_glob | fnmatch ops + filter; glob escape/translate shape | S-05-4 (rows fnmatch, glob), GAP-1 |
| s046_tempfile_names | tempfile presence + TemporaryDirectory lifecycle booleans only | S-05-4 (row tempfile IN-lite) |
| s047_importlib_basics | import_module/reload/find_spec/machinery presence | S-05-4 (row importlib) |
| s048_gcd_bigints | math bigint args, fsum/ulp/nextafter pins | S-05-4 (row math), C-12/C-15/C-16 |

Coverage: every IN_MODULES row (including builtins and genericpath/posixpath) is
mentioned by at least one case — asserted by `test_corpus_covers_every_in_module_row`
(S-05-22 import-smoke + behavior per row group). 48 cases ≥ 25 mandated.

## Antipythany-side gate tests (all `xfail(strict=False)` today — 142 xfails)

| Test | Assertion | Cites |
| --- | --- | --- |
| `test_apconform_antipythany_stdlib_slice[tier-0/adaptive]` ×2 | full slice green per mode | 02.08 Gate, C-31 |
| `test_binary_mode_matches_expected` ×8 (sampled) | built artifact's 3 channels == pinned triple | 02.08 Gate, C-27/C-31 |
| `test_in_module_imports_and_smokes_on_antipythany` ×48 | `import m; print(m.__name__)` conforms | C-05 Verify |
| `test_out_module_build_fails_loud_with_token` ×~60 (non-underscore OUT rows) | nonzero build, token (APX-CEXT/APX-NOV1 by class), module name, file:line:col, no artifact | S-05-2, C-06..C-11 |
| `test_partial_row_enforcement_on_antipythany` ×20 | registry-out names absent (hasattr False); keep-present rows have in-scope siblings; `importlib.metadata` build refused with APX-NOV1 | S-05-4, S-05-2, C-10 |

Green-now fixture tests (284 passed, 2 skipped): table consistency (IN∩OUT=∅, every
table row of spec 05 transcribed, §5.3.3 headline names covered, PARTIAL-row shape),
reference-side subset presence with the filed-whitelist tolerance, OUT modules import
fine on the reference, triple existence, byte-stability re-runs (plain and -S), header
citations, corpus-lint bans, and the ap-conform self-comparison over the whole slice.
Skips: builtins row (covered via corpus s001) and genericpath (GAP-17).

## Ambiguity gaps (filed against spec 05; tests encode the strictest reading or the
corpus-prohibition reading as stated per row, never silence)

- **GAP-1 (fnmatch)**: row lists `filterfalse` in the subset "verified against
  python3.14 -S live dir()"; 3.14.7 `-S` `dir(fnmatch)` has NO `filterfalse`.
  Spec-claim-vs-reference contradiction. Kept in subset (whitelisted as known-absent);
  corpus s045 records the reference truth as a boolean pin.
- **GAP-2 (io / Buffer)**: "`Buffer` (3.14 new) SHOULD be present" — but `io.Buffer`
  is absent on the 3.14.7 `-S` reference itself. A SHOULD against a name the pinned
  reference lacks; implementation MUST NOT be forced to invent it. Not probed.
- **GAP-3 (sys / recursionlimit)**: subset lists `recursionlimit`; the attribute does
  not exist on the reference (only `getrecursionlimit`). Same claim-vs-reality class
  as GAP-1; whitelisted.
- **GAP-4 (sys / _getframe)**: Notes say `sys._getframe` is IN, but it appears only
  in Notes, not the subset column — is a Notes-only name subset-guaranteed? Probed as
  presence-only (corpus s002 includes it in the names list).
- **GAP-5 (re / N flag)**: row lists flag `N` in the parenthesized flag set; 3.14.7
  has no `re.N` (`re.N` was removed in 3.11). Whitelisted spec error; s010 pins
  `hasattr` counts as bytes.
- **GAP-6 (re aliases)**: "`Pattern`/`Match` (+3.14 aliases `RegexFlag`,
  `PatternError`)" — `RegexFlag` exists but there is no `re.Flag` alias on 3.14.7;
  the parenthetical's required alias set is ambiguous. Both listed names probed.
- **GAP-10 (collections.abc count)**: "all 38 ABC names verified" vs the row's own
  enumeration (26 unique names; reference `__all__` has 26, public `dir()` 53
  including builtin-type re-exports). The 38 is unreproducible on any projection we
  found; transcribed the enumerated names only.
- **GAP-11 (collections.abc Buffer)**: row enumerates `Buffer` among the 38; 3.14.7
  `-S` `collections.abc` has NO `Buffer`. Transcribed `Buffer` as an out/registry-
  absent PARTIAL note, not a subset guarantee.
- **GAP-12 (inspect FutureRef)**: Notes put `FutureRef` OUT, but `inspect.FutureRef`
  also does not exist on the reference — the PARTIAL probe (row 14 in PARTIAL_ROWS)
  is therefore a no-op against the reference and can only ever test antipythany.
- **GAP-13 (operator "~73 names")**: row claims "all ~73 names verified in 3.14.7
  dir(operator)"; `-S` shows 57 public (113 with dunders), and the row's example
  names `get` and `call` do not exist on the reference (the real names are `getitem`
  etc.). Count AND example names contradict the "verified" preamble; subset transcribed
  from verified names only.
- **GAP-14 (codecs "BOM consts")**: column says "BOM consts" — enumerated as the full
  `codecs.BOM*` set on 3.14.7 (`BOM_BE_BE`/`BOM_LE_LE` do not exist). Also "full
  Latin/Greek/CJK codec table OUT-of-corpus (documented)": unclear whether this is a
  registry removal or an anchor prohibition (corpus-prohibition reading used; no
  PARTIAL probe).
- **GAP-15 (threading getprofile/settrace/stack_size)**: "OUT-of-corpus" vs S-05-2
  registry fail-loud — corpus-only prohibition (keep-present) reading used, same
  question as GAP-28.
- **GAP-16 (time minimal vs S-05-4)**: row "IN (minimal)" with an explicit subset;
  strictest S-05-4 reading makes every other reference-present name (`asctime`,
  `ctime`, `tzset`, `tzname`, `process_time`, ...) unreachable on antipythany, which
  means `dir(time)` can never be byte-conformant. PARTIAL probe asserts `asctime`
  absent; whether the orchestrator intends subset-closure or superset-allowed here is
  unresolved by the row text.
- **GAP-17 (genericpath importability)**: row "IN (as os.path sources)" with subset
  "per os.path". A bare `import genericpath` under `-S` fails on the reference
  (verified: circular-import AttributeError inside `posixpath`) while `posixpath` and
  `os.path` import fine. Whether genericpath is user-importable in v1 is undetermined;
  the reference-side presence test is skipped, no corpus byte anchors it.
- **GAP-18 (tempfile IN-lite boundary)**: "IN-lite (test-infra)" with a 4-name subset
  "but present for pytest fixtures". pytest fixtures actually need `mkstemp`,
  `NamedTemporaryFile` internals, `SpooledTemporaryFile`... strictest subset →
  `mkstemp` registry-out probe; the practical floor is probably wider.
- **GAP-19 (closure-derived OUTs and the token class)**: S-05-19 marks `statistics`,
  `asyncio`, `uuid`, `shlex`, `hmac` OUT because their closures hit C modules, while
  C-06 demands diagnostics distinguish `c-extension` from `not-in-v1-stdlib`. The
  module named in the user's import (`statistics`) is itself pure-Python — which token
  does the refusal carry? Transcribed the direct-C rows as `cext` (e.g. `zipfile`,
  `http`) and the pure-with-C-closure rows as `nov1` (e.g. `statistics`, `asyncio`,
  `uuid`) — a judgment call spec 05 does not make; C-08 says the named module is the
  one "that resolved to an extension", which for transitive closures could name the
  `_x` dep instead of the import target (C-07 Verify's `json→_json` example supports
  naming the dependency). Needs a spec-05/C-06 ruling.
- **GAP-20 (signal row)**: listed among OUT (S-05-19) yet the parenthetical says "the
  module imports the `_signal` builtin" — implying the C part is runtime-internal and
  `signal` refusal is a scope decision (nov1 reading used) or importable-with-no-
  handlers. Unclear; corpus prohibition of `SIG*` is pinned, import refusal probed.
- **GAP-21 (gc)**: "MUST NOT be importable from corpus code" — corpus-code prohibition
  or registry absence? Either reading is defensible; transcribed registry-out (nov1),
  probed via build-refusal test (xfail).
- **GAP-22 (_thread)**: C-05 explicitly names `_thread` as a native-provided closure
  member while S-05-21 says its import fails loud. Reconcilable (provided to the
  runtime, refused to user imports) but the C-10 class for a user `import _thread`
  is unstated; nov1 used.
- **GAP-23 (random — contract CONTRADICTION, blocking)**: compat-contract C-05's
  named in-v1 floor includes `random`; spec 05 S-05-19 marks `random` OUT-of-v1 with
  "may become IN via PR". The contract wins on conflict (contract §0/§11), which
  makes S-05-19's OUT transcription wrong as written — but then 02.08 must port
  MT19937 byte-exactly (the very reason S-05-19 excluded it). Neither side can be
  silently adopted: `random` is in neither list, `test_random_conflict_is_not_silently_
  scoped` asserts the conflict stands, and one spec-05-by-PR or contract PR is owed
  before 02.08's gate can be declared.
- **GAP-24 (os urandom)**: "no `urandom`-as-corpus" — anchor prohibition (used;
  keep-present + corpus lint) vs os-lite registry exclusion. The row's own subset
  column omits urandom, and S-05-4 would make it unreachable — two readings.
- **GAP-25 (shutil)**: the OUT row's rationale is itself a question mark ("needed by
  pytest for tmpdir cleanup? pytest degrades"). Transcribed OUT per the Status
  column; the Tier-T2 claim depends on the answer.
- **GAP-26 (S-05-4 unreachability vs byte-exactness — the core PARTIAL
  ambiguity)**: S-05-4 demands out-of-subset attribute access raise AttributeError
  "matching CPython's message shape" while C-02/C-27 demand byte-identical channels
  vs the reference — but the reference never raises that AttributeError (the name
  exists there). For every PARTIAL row the two gates cannot both apply to a
  reference-visible surface, and no D-id (D1–D4) covers the divergence. Resolved by
  keeping PARTIAL assertions on the antipythany-only side (direct probe tests, xfail)
  and OUT of the byte-diffed corpus. Needs a divergence-register ruling.
- **GAP-27 (PARTIAL failure surface — runtime vs compile-time)**: the task/leaf
  wording offers "ModuleNotFoundError-with-APX-NOV1 or ImportError exactly as spec 05's
  PARTIAL definition says"; spec 05 S-05-2 says NO runtime `ImportError` substitute —
  fail-loud is compile-time (or import-time only for dynamically-computed names,
  C-07). Strictest reading (S-05-2): compile-time refusal with APX-NOV1 for
  `importlib.metadata` and OUT submodules; runtime ModuleNotFoundError is treated as
  a conformance bug.
- **GAP-28 ("OUT-of-corpus" vs "OUT")**: rows mix the two phrasings (functools
  `namedtuple`, re `Scanner`, ast `main`, inspect `markcoroutinefunction`/`FutureRef`,
  contextlib `chdir` "IN but corpus-prohibited"). `chdir`'s explicit "IN but" proves
  corpus-prohibition ≠ registry-removal, so bare "OUT" (ast `main`) was read as
  registry-out and "OUT-of-corpus" as keep-present — a wording-based split the spec
  should state outright.
- **GAP-29 (sys.monitoring)**: Notes mark `sys.monitoring` OUT, yet Plan-02 tier-1
  tooling (02.05/OSR, T-12 parity) and profilers commonly expect it; whether OUT here
  is registry-level or corpus-level, and what C-10 token an attribute probe gets, is
  unspecified. Probed as registry-out (strictest, per Notes' plain "OUT").
- **GAP-30 (importlib util/machinery reachability)**: on the reference, bare
  `import importlib` does NOT bind `importlib.util`/`importlib.machinery` (verified
  live: AttributeError); the subset lists them as `importlib.X` names. Whether the v1
  registry must auto-bind those submodules is unspecified (corpus s047 imports them
  explicitly; the reference behavior is byte-pinned for the explicit form only).
- **Provenance (C-01)**: every expected byte above was recorded from
  `/opt/homebrew/bin/python3.14` = **3.14.7**, not the C-01-pinned 3.14.0, because no
  3.14.0 build is installed on this host. Per C-01 the expectations are provisional;
  re-generate (or diff-verify) the triples on the pinned build when available — same
  open item as T-05 GAP-7 / T-06 precedent, not a T-11 blocker.

## Run

```
/opt/homebrew/bin/python3.14 -m pytest tests/t11_stdlib -q
```

Expected today: `284 passed, 2 skipped, 142 xfailed`, zero collection errors.
After 02.08 lands: xfails flip to passes; any FAIL is a 02.08 conformance bug
(strict=False keeps xpasses non-fatal so the orchestrator can triage; do not convert
to strict=True until the gate run, per tests/README.md §4).
