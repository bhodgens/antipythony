# antipythony bughunt 2026-11-26 - ADDENDUM, auditors D through K

This file continues .hermes/audits/2026-11-26-bughunt-report.md. It holds the
findings of the 8 auditors of dispatch deleg_850fed31.

HEAD when findings were verified: e79eec0 (2026-11-27 00:29).
Binary used: target-0304c/debug/antipythony, built 2026-11-27 00:27.
Reference: CPython 3.14.7 (/opt/homebrew/bin/python3.14).
Window: c51d78d..e79eec0, 1112 non-merge commits.

## 6. AUDITOR FINDINGS D THROUGH K (parent-verified)

Verification method. For each finding I ran the reproduction on the real
binary and on CPython 3.14.7 and compared stdout, stderr and the exit code.
For a wasm finding I built the module with `antipythony build --wasm` and ran
it with `antipythony host`. For a JIT finding I set APX_TIER=jit and
APX_SYNC_TIERUP=1. For a C-API finding I re-ran the auditor's compiled C
probes (built 2026-11-27 00:27 from the same source as the binary) in my own
copy of the probe directories. For a gate finding I read the source and the
git history. I then read the code at HEAD for each site.
Harness and logs: ~/.git/worktrees/bughunt-verify/ (verify_wave2.py,
wave2run1.log, wave2manual/, ce/).

Severity is my grade. It can differ from the grade of the auditor.

Self-audit of counts. Auditors D through K reported 62 findings
(D 12, E 2, F 11, G 12, H 12, I 12, J 13, K 6). I count each finding once.
H1 is the same defect as BL-2 of the main report and is counted here as that
finding. I1 is one finding with two reproduced shapes.

Per auditor, the number of findings and how each was verified:

| Auditor | Findings | Run | Source and history | Source only, not run | Partial |
|---|---|---|---|---|---|
| D wasm | 12 | 12 | 0 | 0 | 0 |
| E native JIT | 2 | 2 | 0 | 0 | 0 |
| F frontend | 11 | 10 | 0 | 0 | 1 (F8) |
| G C-API | 12 | 9 | 0 | 3 (G8, G10, G12) | 0 |
| H gates | 12 | 0 | 11 | 0 | 1 (H9) |
| I import | 12 | 11 | 0 | 1 (I11) | 0 |
| J corpus | 13 | 0 | 13 | 0 | 0 |
| K forensics | 6 | 3 (K1, K4, K5) | 0 | 0 | 3 (K2, K3, K6) |
| Sum | 80 auditor rows -> 79 unique findings | 47 | 24 | 4 | 5 |

Note on the sum: 12 + 2 + 11 + 12 + 12 + 12 + 13 + 6 = 80 rows, and H1
duplicates BL-2, so 79 unique findings. The per-column sum is
47 + 24 + 4 + 5 = 80. Each finding is in one column.
47 + 24 = 71 findings have a run or a source read behind them. 4 are source
reads that the auditor also marked unverified (G8, G10, G12) or that I did
not run (I11). 5 are partial or not re-checked (F8, H9, K2, K3, K6).

For the J column "source and history": I read J1, J2, J3, J4, J6, J7, J9
first-hand. For J5, J8, J10, J11, J12, J13 I read the cited lines only. I
did not re-run the 61 expected-output triples that the auditor compared with
CPython 3.14.7. No J finding was run.

### 6.1 Wasm backend (auditor D) - 12 findings, all reproduced

LEAF LANDED 2026-11-27: D7 and D4 are FIXED on main (merge 3141515, leaf
commit b1066b5, branch fix-wasm-super-zeroarg). Record:
plans/antipythony/03-wasm-backend/NEXT-WASM-SUPER.md. D7: zero-argument
super() in a method now gives the CPython result. D4: a method overridden in
a subclass is no longer bound into the base class. The leaf also fixed four
more defects on the same call path (super-object attribute lookup, the bound
first argument of an attribute call, an attribute call with 3+ arguments, and
an arm that returned inside an expression). Verified on merged main, own
CARGO_TARGET_DIR: classes_exceptions_wasm_gate + 3 other wasm gates + unit
tests = 7 passed / 0 failed; corpus_traceability 606 passed / 1 skipped;
c021 PASS (the RuntimeError path for a nested function is intact). RED on
base for the 4 new cases (CLW009..CLW012): 17 passed / 4 failed.
D1, D5, D6, D8, D11, D12 are FIXED on main (merge 411aa97, leaf
fix-wasm-findings, record plans/antipythony/03-wasm-backend/
NEXT-WASM-FINDINGS.md; gates green on the leaf tree; live probes on the
merged tree fdc86cb pass for the pinned shapes). CORRECTIONS from the
parent's post-merge probe sweep (.hermes/audits/
2026-11-27-wasm-followup-probes.md):
- D3 is PARTIAL, not FIXED: the leaf removed the trap (big_bits n==0
  guard + big_from_i64 wrap-abs, both present and real), but the value
  is corrupted upstream before bigint code runs - the LitBigInt arm
  (lower.rs ~4060) wraps every big literal to i64, and binop's POW loop
  uses plain i64 mul with no overflow check, so `2**63` prints 0, not a
  trap, not the right number. Silent data corruption, exit 0. Filed as
  GitHub issue #31; a fix leaf (wasm-bigint) is in flight.
- D12 is PARTIAL: list/tuple membership, str ORDERING, and tuple except
  targets work (CF020 byte-exact), but `'a' in 'abc'` (str-in-str
  membership) still traps, and a missing attribute is still wrong: the
  leaf's AttributeError raise is silently swallowed (rc=0, no output,
  no traceback - caught and uncaught both). Filed in the followup-probes
  doc as W2/W3; a fix leaf (wasm-w2w3) is in flight.
- D2: diagnosed-deferred by the leaf, confirmed live at scale by the
  parent sweep (see issue #31). Fix = the same limb core as D3's issue.
- D9: FIXED by a different session's float-repr helpers (print(1.5)
  gives 1.5 on the merged tree; live probe).
- D10: still open (2**53 ordering), priced in NEXT-WASM-FINDINGS.md.
NEW base defects found by the wasm-super leaf, now FIXED on main (merge
fdc86cb, leaf fix-wasm-strplus, record NEXT-WASM-STRPLUS.md): `str +
str` concatenation, `str * int` repetition, the nested-print buffer
corruption, and print(function object) (deterministic no-address shapes,
a documented divergence from CPython's address repr). 6 new corpus cases
CLW013..CLW018 byte-exact; live probes on the merged binary pass.

The wasm backend compiles Python to a WebAssembly GC module. Each finding
below changes the program result or crashes the module. The bytecode-mode
interpreter gives the CPython result for each of these programs.

| ID | Sev | Site at HEAD | Reproduction | wasm result | CPython 3.14.7 |
|---|---|---|---|---|---|
| D1 | HIGH | prelude.rs:785-917 `truthy`, :464-603 `bool_obj` | `b = False` then `if b:` | takes the `then` branch | takes the `else` branch |
| D2 | HIGH | prelude.rs:2905 (mul), :2913 (shl); lower.rs:6103-6109 | `3037000500 * 3037000500` | -9223372036709301616 | 9223372037000250000 |
| D3 | HIGH | prelude.rs:365-378, :3507-3514 | `print(-(2**63))` | trap "wasm host error" | -9223372036854775808 |
| D4 | HIGH | lower.rs:5387-5392 | `class A: def g` then `class B(A): def g` | `A().g()` gives "B.g" | "A.g" |
| D5 | HIGH | lower.rs:5954-5978, :5847-5851 | `raise` inside `except` | trap | re-raises ValueError |
| D6 | HIGH | prelude.rs:1860-1884 | `x = 1.5; x == x + 0.0` | False | True |
| D7 | HIGH | lower.rs:6444-6464 | `super().__init__()` | RuntimeError | works |
| D8 | HIGH | prelude_classes.rs:731-734 | infinite recursion | trap | RecursionError |
| D9 | MED | prelude.rs:2317-2331 | `print(1.5)` | `1` | `1.5` |
| D10 | MED | prelude.rs:1723-1736, :1089-1092 | `9007199254740993 == 9007199254740992` | True | False |
| D11 | MED | lower.rs:5886-5894 | `finally` after a raise | `finally` body skipped | body runs |
| D12 | MED | prelude.rs:1801, :2958, :3032; lower.rs:5961 | `1 in [1, 2]`, `1 // 0` | trap | True, ZeroDivisionError |

Notes on single findings:

- D1: `type_sel` gives a bool the selector 2. `truthy` tests the selectors
  0, 1, 3, 4, 5, 6, 7, and 8 only. A bool keeps the default value 1, which
  means true. A stored False is true in every `if`, `while`, `not`, `and`,
  and `or`. `while b:` with a stored False never ends.
- D2: the multiply is a plain i64 multiply with no overflow check. A big
  literal is cut to 64 bits at lower.rs:6104-6108, so `2**64` is 0.
- D3: the absolute value of the i64 minimum wraps to 0, which gives a bigint
  with no limbs. The next read of limb 0 is an out-of-range array read.
- D4: a method in a class body is found by the LAST def with that name in
  the whole module (`rposition`), not by its position in the class.
- D5: a bare `raise` reads a global that is set only on generator paths.
  Outside a generator the global is null, and the raise function traps.
- D6: the float equality arm requires an int operand on one side. Two float
  operands fall through to "false". Only reference equality gives True.
- D7: the code comment names this as an intended corpus observable (c021).
  The corpus pins a divergence from CPython. This is a product decision for
  the owner: keep the pin, or implement zero-argument super().
- D8: the frame depth counter is incremented (prelude_classes.rs:731-734) and
  no code compares it with a limit.

### 6.2 Native JIT (auditor E) - 2 findings, both reproduced

Tier 1 is the compiled code tier. APX_TIER=jit forces it.

| ID | Sev | Site at HEAD | Reproduction | JIT result | CPython 3.14.7 |
|---|---|---|---|---|---|
| E1 | HIGH | runtime.rs:582-586 `h_match_exc` | `try: raise ValueError('x')` then bare `except` | SystemError: MatchExc region mismatch: 0 | the handler runs |
| E2 | HIGH | clif.rs:2275-2282 | `6 / 3` in a hot function | `2` (an int) | `2.0` |

- E1: the field `match_region` has exactly three references in the workspace:
  the declaration (runtime.rs:75), the init to u32::MAX (runtime.rs:106), and
  the comparison (runtime.rs:582). No code writes it. So a bare `except` in
  JIT-routed code always raises SystemError. The bytecode mode is correct.
- E2: `BinOp::Div` shares the arm of `BinOp::FloorDiv`, which is an integer
  division. The auditor used CPython 3.15.2 as reference. I re-ran the probe
  against CPython 3.14.7 and the bytecode mode: `2.0 0.3333333333333333 -3.5`.
  The JIT gives `2 0 -3`. E2 reproduces after the tier-up, which needs a warm
  loop. The first calls are correct.

### 6.3 Frontend (auditor F) - 11 findings, 10 reproduced, 1 partial

| ID | Sev | Site at HEAD | Reproduction | result | CPython 3.14.7 |
|---|---|---|---|---|---|
| F1 | HIGH | surrogates.rs:65; crash at ap-run format.rs:85 | `f"{x:\ud800>5}"` | abort rc -6 "invalid value for `char`" | 5 |
| F2 | HIGH | lowering.rs:1382 | `0x1_0000_0000_0000_0000 % 1000` | SystemError: bad bigint literal | 616 |
| F3 | HIGH | lowering.rs:500-527; runtime in ap-run | list comp two function scopes deep reads a grandparent name | NameError | [10, 10] |
| F4 | MED | lowering.rs:432-441 | `nonlocal x` through a scope that does not bind x | 1 | 2 |
| F5 | MED | surrogates.rs:584-601 | `len(rf"\ud800")` | 1 | 6 |
| F6 | MED | lib.rs:259-294 | `if 1:` then `return 1` at module scope | accepted | SyntaxError |
| F7 | MED | lib.rs:173-181 | traceback after a multi-line def | `<module>` line 3, wrong source line | `<module>` line 4, `g()` |
| F8 | MED | lowering.rs:1690-1698, :1640-1667 | see the note | see the note | see the note |
| F9 | MED | lowering.rs:1552-1554; runtime in ap-run | `x = 2; del x; print(x)` in a function | prints the global 1 | UnboundLocalError |
| F10 | LOW | lowering.rs:731-735 | `class C(**{...})` | "lowering bug" | class |
| F11 | LOW | lib.rs:33-44 | walrus in a comprehension iterable | accepted | SyntaxError |

Notes on single findings:

- F1: the surrogate shim builds a Rust String with `from_utf8_unchecked`
  from bytes that hold a lone surrogate. Those bytes are not valid UTF-8.
  Every later `.chars()` call on that String is undefined behavior in Rust.
  The debug build aborts. A release build can give a wrong result with no
  error. This is the same root as B2 in the main report.
- F3: the scope table is correct (auditor read it with APX_SCOPE_DEBUG=1).
  The defect is in the runtime of ap-run, which builds the frame chain of
  the comprehension. A lambda, a def, or a generator expression in the same
  position works.
- F8 PARTIAL. Confirmed: `case [*rest, q]` on `[1, 2, 3]` binds
  `rest = [2, 3]` and `q = 1` (CPython: `[1, 2]` and 3). Confirmed:
  `[1, *rest, 3]` never matches. NOT confirmed: the claim that
  `case PATTERN as name` drops the capture. My probe `case (1, t):` on
  `(1, 2)` binds `t` to 2 on both interpreters.
- F2, F4, F9, F10: the cited lines are older than the window. They are real
  defects in files of this scope.

### 6.4 C-API layer (auditor G) - 12 findings: 9 reproduced, 3 source-only

The crate ap-ce emulates the CPython C API so that a compiled extension
module can load. The probes are C files compiled with `antipythony
build-ext`.

| ID | Sev | Site at HEAD | Reproduction | result |
|---|---|---|---|---|
| G1 | HIGH | exports_pydantic.rs:1317-1321 | `PyObject_VectorcallMethod` | abort rc -6 "misaligned pointer dereference ... 0x2" at value_conv.rs:28 |
| G2 | HIGH | exports_method_mem.rs:345-366 | contended `PyMutex_Lock`, then unlock from the main thread | hang (killed at 20 s); the uncontended control passes |
| G3 | HIGH | value_conv.rs:92; ce_bridge.rs:44-50, :468-482 | two C calls that convert the same instance | the second call raises SystemError "PyObject_GetAttr: NULL object"; a call passes 0 arguments |
| G4 | HIGH | exports.rs:188, :275, :345; exports_containers.rs:1251; exports_long_gc.rs:668 | negative or huge length from the extension | abort "null pointer dereference" (exports.rs:144), abort "capacity overflow", segmentation fault rc -11 |
| G5 | HIGH | exports.rs:387 (pattern at :144, :428) | 4 threads share one dict handle | abort "RefCell already borrowed" at exports.rs:387:63, 3 runs of 3 |
| G6 | HIGH | state.rs:204-208; slots.rs:297-301 | `PyErr_NewException("othermod.Box", ...)` next to a registered type "p9.Box" | the new class is not a subclass of ValueError; `except` on it raises TypeError |
| G7 | MED | ce_bridge.rs:181-193 | C function sets an error and returns a value | the value is returned; the next harmless call raises SystemError |
| G8 | MED | ce_bridge.rs:640-646 | `*out.rows.add(i) = r` on uninit memory | SOURCE ONLY: the assignment drops an old value that is garbage memory |
| G9 | MED | exports.rs:538-557 | `apx_PyExc_ValueError() == apx_PyExc_ValueError()` | 0 (a new object on each call); Py_None is stable |
| G10 | MED | ce_bridge.rs:300-306 | tp_new returns Py_None or a cached instance | SOURCE ONLY: the return value is discarded |
| G11 | LOW | exports_pydantic.rs:499; exports.rs:144, :387, :428 | a null argument | abort "null pointer dereference" |
| G12 | LOW | payloads.rs:43-64 | instances with a payload, then GC | SOURCE ONLY: the payload block is never freed |

Notes on single findings:

- G2 is a process-wide deadlock. One global registry mutex is held while the
  code locks it again in the wait loop. After that, every PyMutex call in the
  process blocks.
- G4: an abort inside an `extern "C"` function cannot be caught by
  `catch_unwind`. CPython returns NULL with MemoryError or SystemError.
- G5: the control run (4 threads, one handle per call) passed with 0 errors.
  The abort needs a handle that lives across threads: a module handle, a
  cached handle, or an extension static.
- G8, G10, G12: the auditor marked these as unverified. I read the cited
  lines and agree that they are defects by the Rust language rules. I did
  not run them, because none gives a deterministic failure.
- The pydantic extension was not loaded in this wave. The G findings show the
  defect on a small C probe, not on pydantic itself.

### 6.5 Gates, CI and test helpers (auditor H) - 12 findings: 11 confirmed by source and history, 1 partial

| ID | Sev | Site at HEAD | Defect |
|---|---|---|---|
| H2 | HIGH | crates/ap-run/tests/re_fuzz.rs:884-889 | The PR lane of the regex fuzz test maps every divergence to Ok. ci.yml does not set AP_FUZZ_STRICT. A new regex regression is green in `cargo test --workspace`. |
| H3 | HIGH (merged with H2) | .github/workflows/nightly-fuzz.yml:109, :119, :174 | The strict lane is `continue-on-error` and the summary step ends `exit 0`. No step can fail the job. KNOWN GAP: the file header says "RED BY DESIGN today" and names the removal of continue-on-error as the exit condition. |
| H4 | HIGH | .github/workflows/ci.yml:72-79; scripts/unsafe_report.sh | The unsafe ratchet is a diff of two generated files. docs/unsafe-baseline.txt was edited 10 times in the window. The commit that added the gate (6880199) raised the count from 229 to 235. `unsafe impl` and `unsafe trait` (42 sites) are not counted. Test directories are not counted. |
| H5 | HIGH | crates/xtask/src/stdlib_provenance.rs:619 | Fallback to `python3` or `python` with no version check. On a runner with Python 3.12 the vendored tree is compared with the wrong standard library. |
| H6 | MED | crates/ap-cli/tests/hybrid_gate.rs:490-492, :511-526 | Without python3.14 the CPython re-check is skipped with no SKIP line. The suite does not honor AP_REQUIRE_REFERENCE. |
| H7 | MED | crates/ap-cli/tests/wasm_build_gate.rs:137-151, :195-225 | The wasmtime arm and the byte-identity arm are skipped with no notice when the tool or the directory is absent. `which wasmtime` is empty on this host and CI does not install it. |
| H8 | MED | crates/xtask/src/cli_docs.rs:136-145 | A binary that is missing or exits non-zero on `--help` renders NOT BUILT. A committed NOT BUILT doc passes the check. |
| H9 | MED, PARTIAL | crates/ap-cli/tests/common/mod.rs:146-166 | `flock(LOCK_EX)` with no time limit on the per-probe lock. CORRECTED: the rust job has `timeout-minutes: 180` (ci.yml:214), so a wedged lock stops after 3 hours, not never. |
| H10 | MED | crates/xtask/src/pins.rs:683-708 | `pins --check` is not wired into any workflow. It hashes the .out file only, never .err or .rc. |
| H11 | MED | tests/t08_tier1/test_t08_tier1.py:99, :115 | xfail reason "awaits 02.05 tier-1". AGENTS.md says 02.05 is complete. tier_mode_differential.rs:311-332 prints the known wrong value as INFO with no assert. |
| H12 | LOW | crates/ap-cli/tests/stdlib_modules_differential.rs:57-67 | Exit code and stdout are compared. Stderr is not compared. |
| H1 | (same as BL-2) | crates/ap-cli/tests/wasm_build_gate.rs:116, :239 | See BL-2 in the main report. |

I did not run any gate command for this scope.

### 6.6 Import system and embedded stdlib (auditor I) - 12 findings: 11 reproduced, 1 source-only (I11)

I1 has two shapes. Each has its own row.

| ID | Sev | Site at HEAD | Reproduction | result | CPython 3.14.7 |
|---|---|---|---|---|---|
| I1 | HIGH | importer.rs:1021-1043 | a `_collections_abc.py` on sys.path that imports `collections.abc` | stack overflow abort rc -6 | completes |
| I1b | HIGH | same | `_collections_abc.py` with a `print` in its body | body printed 2 times | not printed (frozen module) |
| I2 | HIGH | importer.rs:1841-1848 | `__package__ = None` then `from . import mod` | ImportError | works |
| I3 | HIGH | importer.rs:2242-2251 | embedded mode, `import pathlib` | AttributeError on genericpath._splitext | works |
| I4 | MED | importer.rs:1794-1812 | `del sys.modules[x]` then `import x` | stale module, not in sys.modules; a None entry is ignored | re-import; ModuleNotFoundError |
| I5 | MED | builtins.rs:6033-6046 | `__import__('pkg.sub.deep')` | leaf module | top package |
| I6 | MED | interp.rs:15516-15522; importer.rs:872-881 | `__builtins__` in `__main__` | NameError | module |
| I7 | MED | builtins_modules.rs:22336 | `importlib.reload(m)` | no-op; `reload(42)` accepted | re-executes; TypeError |
| I8 | MED | importer.rs:436-442 | embedded mode, package `__file__` | `.../encodings.py`, does not exist | `.../encodings/__init__.py` |
| I9 | MED | importer.rs:1581-1597 | namespace package `__spec__` | parent '', locations [], no loader | correct spec |
| I10 | MED | builtins_modules.rs:22484-22575 | `importlib.util.find_spec('json.decoder')` | None | a ModuleSpec |
| I11 | LOW | importer.rs:397-406 | `embedded_store_active()` | SOURCE ONLY: compares a canonical path with a non-canonical literal. The binary holds the literal `/Users/caimlas/git/antipythony/crates/ap-run/../../vendored_stdlib`. | n/a |
| I12 | LOW | interp.rs:1757 | traceback of a failed import | line number of the NEXT statement | line of the import |

Notes on single findings:

- I1: `sys_modules_insert` runs AFTER the module body (line 1042). So the
  body can import the module again. A body that raises is swallowed by the
  `.is_ok()` at line 1029.
- I3: the embedded mode is the mode of a standalone binary. In that mode
  `pathlib`, `asyncio`, `logging.config`, `typing` and `string` do not
  import. I reproduced this with ANTIPTYHONY_STDLIB set to an empty
  directory. I did NOT build a standalone binary. Second defect in the same
  run: the traceback reports line 251 of a 2-line script.
- I3, I8, I11 are tied to the embedded store leaf ef2ccdd, which is in the
  window.

### 6.7 Test corpus (auditor J) - 13 findings: 7 confirmed by first-hand read, 6 by a read of the cited lines

| ID | Sev | Site at HEAD | Defect |
|---|---|---|---|
| J1 | HIGH | tests/t04_fastapi_reach/test_t4_reach_regress.py:25-31 | `_bin()` looks in target-t4, target/debug, and PATH. It never reads target-0304c. All 7 tests of the file call it. |
| J2 | HIGH | tests/test_a1_rustpython_backfill.py:81-82; t11:96; t05:81-83; t10:102-104 | Fallback to target/debug for ap-run-eval and ap-conform. CI does not set AP_RUN_EVAL or AP_CONFORM. |
| J3 | HIGH | tests/test_a1_rustpython_backfill.py:659-685 | A non-strict xfail wraps a subprocess call with a 120 s limit. A hang is reported as xfail. The file has no handler for TimeoutExpired (0 hits). |
| J4 | MED | crates/ap-conform/src/lib.rs:212, :258 | A child killed by a signal gives exit code None on both legs. The compare gives Pass when stdout and stderr are also equal. |
| J5 | MED | tests/test_a12_mined_corpus.py:113-114 | A pin holds the absolute path /Users/caimlas/git/antipythony with no masking, under a non-strict xfail. |
| J6 | MED | tests/test_a12_mined_corpus.py:159-176 | The test asserts that xfail markers MUST exist. It goes red when a gap is fixed. |
| J7 | MED | tests/t10_threading/test_t10_threading.py:107, :342-353 | xfail reason "awaits 02.07 native threading". 02.07 is complete. |
| J8 | MED | tests/t11_stdlib/test_t11_stdlib.py:403-404 | A non-JSON summary gives `{}`. Then only `failed == 0` is asserted. The t10 twin asserts all three counts. |
| J9 | LOW | tests/t11_stdlib/test_t11_stdlib.py:408-410 | The AoT leg is xfail for 8 parameters, also for the cases that pass. |
| J10 | LOW | tests/bench/primitive/fix_bytecode.py:82 | Accepts any python3. Its filter drops a stderr line with the text "return 2". |
| J11 | LOW | tests/README.md:11-16 | The README names .out/.err/.rc and a generator. 61 cases use .stdout/.stderr/.exit. Most families have no generator. |
| J12 | LOW | tests/t07_gc/_gen_expected.py:10, :19 | Two absolute paths as literals. |
| J13 | LOW | tests/test_a1_rustpython_backfill.py:612-621 | Host-conditional skips with no Linux sibling pin. |

Notes on single findings:

- J1 and J2: I read the code and the CI environment. The claim for CI is
  derived, not run: CI is ubuntu, the checkout has no target/debug, and the
  tests skip. On THIS host, target/debug/antipythony and
  target/debug/ap-run-eval exist with mtime 2026-11-24, so the skip does not
  fire here. target/debug/ap-conform does not exist here. A pytest run was
  not permitted for this wave. The count of skipped tests on CI is NOT
  measured.
- J3: I read the code. A hang is an exception inside a non-strict xfail
  test, which pytest reports as xfailed. I did not run it.
- J4: I read the code at the three cited lines. A signal kill on both legs
  gives Pass. A one-sided kill gives Fail. I did not run it.
- Auditor J reports that all 61 expected-output triples added in the window
  match CPython 3.14.7 byte for byte. I did not re-run the 61 cases.

### 6.8 Build forensics (auditor K) - 6 findings: 3 confirmed first-hand, 3 not confirmed

| ID | Sev | Claim | Parent check |
|---|---|---|---|
| K1 | MED | Linux files come from cargo run natively in an ARM64 Linux container with the repository at /repo. | CONFIRMED: ap-host.d names /repo/; 675 dep-info files name /repo/; no cross config on the Mac. The exact command is not recorded. |
| K2 | LOW | probe-bin is a throwaway crate crates/probe-bin, never in git. mtime 16:02:56. | PARTIAL: mtime 16:02:56 and the missing crate are confirmed. The purpose (a probe for a GNU-ld SIGSEGV) is the auditor's inference from commit dd9ef7c. I did not confirm it. |
| K3 | MED | Built by the primitive-perf session after its commit 4141a86 at 19:10. | PARTIAL: commit e8752ae names the container verification. The exact time window is the auditor's inference from fingerprint mtimes. |
| K4 | LOW | Exactly 2 ELF files at the top level of target-0304c/debug. | CONFIRMED: ap-host and probe-bin. |
| K5 | HIGH | The files are hard links (link count 2) and the bin unit was never re-linked. | CONFIRMED: link counts 2, 2, 2. The fingerprint dir ap-host-ada12ad071f4ce90 has mtime 2026-11-24 19:14. |
| K6 | LOW | No aarch64-unknown-linux-gnu directory. One abandoned x86_64 directory. | NOT re-checked by the parent. |

The baseline section BL-1 of the main report holds the corrected text.

## 7. DISCLOSURE LEDGER, auditors D through K

1. Each probe ran once. I ran G5 three times (3 aborts). No probe ran under
   a race detector.
2. The wasm probes ran through `antipythony host`. The standalone ap-host
   binary is a Linux file on this host and did not run (BL-1).
3. The G findings are shown on small C probes, not on the pydantic
   extension. The extension was not loaded.
4. G8, G10, G12 are source reads. No run shows the defect.
5. The H findings are source reads plus git history. I did not run xtask,
   the fuzz lanes, or a CI job.
6. The J findings for CI (J1, J2) are derived from the code and the CI
   environment. No pytest run measured the skip count.
7. Auditor E used CPython 3.15.2 as its reference. I re-ran E1 and E2
   against CPython 3.14.7.
8. Auditor D listed items in its notes that it did not report, because of
   the cap of 12. I did not verify those: a chained comparison that evaluates
   the middle operand twice, a wrong hash for negative ints, a sum over
   negatives that gives 0, and len(str) that counts bytes.
9. K6 and the purpose of probe-bin (K2) are not confirmed by the parent.
10. A release build of the interpreter was not tested. F1 and B2 are
    undefined behavior in Rust: a debug build aborts, and a release build can
    give a wrong result with no error.
11. Scratch directories stay on disk outside the repository:
    ~/.git/worktrees/bughunt-verify/ (harness, logs, probe copies),
    ~/.git/worktrees/bughunt-wasm-h4uy870m, and the 6 auditor scratch
    directories ~/.git/worktrees/bughunt-{ce-G-*,front-F-*,imp-I-*,native-E-*,
    wasm-D-*,logs}. The ce directory holds compiled .so files.
