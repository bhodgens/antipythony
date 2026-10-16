# ANTIPYTHANY Compatibility Contract

Leaf: 00.03 (`plans/antipythany/00-foundation-contract/orchestrator.md`).
Status: normative. This document is the compatibility contract between antipythany
and its reference implementation. It is an input to the spec pack (00.05) and one of
the two documents test agents read together with `docs/spec/` (see
`plans/antipythany/tests/README.md`).

Relaxation rule (locked): Plan 10 / v2 research leaves may relax a clause marked
`v1-locked` ONLY by a PR that amends this file, with clause-id reasoning. No code
may relax a clause silently.

## 0. How to read this document

- **MUST / MUST NOT / SHOULD / MAY** carry RFC-2119 force. A `MUST` violated in any
  mode (tier-0, tier-1 JIT, AoT binary, wasm) is a conformance bug.
- Every clause `C-nn` has a **Verify** line: a concrete assertion a test agent can
  write from this document plus the spec pack alone, without reading implementation
  code. Where a clause is about the *absence* of behavior, the Verify line says how
  to observe the absence.
- "The reference" means CPython 3.14.0 as pinned in C-01. Where this document gives
  example expected outputs, they are the reference outputs; the spec pack restates
  them with full generality.
- Clause areas: C-01..C-03 reference implementation; C-04..C-06 supported subset;
  C-07..C-10 fail-loud extension rule; C-11..C-13 integer semantics; C-14..C-18
  destruction timing; C-19..C-22 float repr; C-23..C-28 exit/stderr contract;
  C-29..C-30 implementation transparency.

## 1. Reference implementation

### C-01 — Reference is CPython 3.14.0, pinned (v1-locked)
The reference implementation for all differential conformance is **CPython 3.14.0,
exact patch version**. Byte-exact conformance is defined only against this build.
The harness invokes it as `python3.14` from `PATH` (00.04); that binary MUST be a
3.14.0 release build (not a prerelease, not 3.14.x for x != 0). If the exact build
is unavailable, conformance runs MUST fail with a harness error rather than silently
comparing against a different version.

**Verify:** run `python3.14 -VV` and assert the output starts with
`3.14.0 (tags/v3.14.0` or matches CPython 3.14.0 release identification
(`sys.version_info == (3, 14, 0, 'final', 0)`); assert `ap-conform` refuses to run
(`ap-conform run` exits nonzero with a version-mismatch diagnostic) when the probe
reports any other version.

### C-02 — Tie-break rule for undefined or ambiguous behavior
Where antipythany behavior is not fixed by this contract or the spec pack, the
reference's observed behavior on the pinned 3.14.0 build is normative by default.
A divergence from the reference is a bug UNLESS it is (a) listed as a documented
divergence in §7 of this file, or (b) a timing-only difference (C-15, C-29). Any
new divergence MUST be added here by PR before it may ship.

**Verify:** for any corpus case, `assert antipythany_output == cpython_output` holds
byte-wise (C-28) unless the case header cites a divergence clause id. A CI check
scans corpus case headers: every cited divergence id exists in §7.

### C-03 — One Python version at a time
v1 supports exactly one Python language version: 3.14 (master.md, locked decision).
The compiler MUST reject source that the pinned ruff parser (`PythonVersion::PY314`)
rejects, with a byte-precise parse diagnostic (01.01), and MUST NOT accept
future-version-only syntax. `sys.version_info` reported by the runtime MUST be
`(3, 14, 0, 'final', 0)`.

**Verify:** (1) a program using 3.15-or-later syntax (if any reserved grammar is
probed) fails to compile with a parse diagnostic naming the line and column;
(2) `python -c "import sys; print(sys.version_info)"` under antipythany prints
`sys.version_info(major=3, minor=14, micro=0, releaselevel='final', serial=0)`
identically to the reference; (3) the parser rejects CPython 2-style
`print "x"`.

## 2. Supported subset, v1

### C-04 — Pure-Python user programs are supported
Any user program written in the pure-Python subset of the 3.14 language (grammar +
semantics covered by spec pack sections 01–04), whose complete transitive static
import closure satisfies C-05, MUST compile and MUST conform to the reference on
stdout, stderr, and exit code (C-28). "Pure Python" here means: every module in the
closure executes with no compiled-extension dependency.

**Verify:** each corpus slice case (values, control-flow, classes, exceptions,
generators, stdlib-v1) is by construction pure-Python and MUST pass ap-conform.
Conversely, a program containing no extension import MUST NOT be refused by the
compiler (no false rejections): assert compile success on the stdlib-v1 import test
set of C-06.

### C-05 — v1 stdlib = stdlib modules whose import closure needs no C extension
A standard-library module is in the v1 supported set iff its transitive import
closure (static `import`/`from` analysis at compile time) contains only (a)
pure-Python stdlib modules and (b) modules the antipythany runtime provides natively
as part of the v1 allow-list — including `builtins`, `sys`, `_thread`, and the
internals they pull in. The exact allow-list and the native-provided list are
enumerated in spec pack section 05 (`docs/spec/05-builtins-stdlib-v1.md`); this
clause fixes the *rule*, spec 05 fixes the *list*. If spec 05 and this rule ever
disagree, spec 05 is corrected by PR — §0 of spec 05 states this.

**Verify:** take the module list from spec 05. For each listed module `m`: compiling
`import m; print(m.__name__)` succeeds and output matches the reference. For each
stdlib module NOT listed and known extension-only (e.g. `_socket`, `_ssl`, `_json`,
`_sqlite3`, `_ssl`): compiling `import <name>` fails per C-07.

### C-06 — Support is an allow-list, not a negative inference
v1 support decisions are made only by the positive allow-list (C-05 / spec 05).
The compiler MUST NOT accept an import merely because "it is pure Python and we did
not think about it": a module not present in the v1 registry produces the same
compile-time failure class as an extension module (C-07), with the diagnostic
naming the reason (`not-in-v1-stdlib` vs `c-extension`). Third-party packages are
out of scope for Plans 00–02; they enter the closure check only once the packaging
plan (04.01, uv integration) lands, and they are subject to the same extension rule.

**Verify:** compile `import urllib.robotparser` and `import statistics` (pure-Python,
in-v1 candidates) — both accepted once spec 05 lists them; compile `import quixotic`
(nonexistent) and a stdlib module deliberately excluded from spec 05 — both rejected
at compile time with diagnostics distinguishing `module-not-found` from
`not-in-v1-stdlib` from `c-extension` (assert the reason token appears).

## 3. Fail-loud rule for C extensions

### C-07 — Any import resolving to a C-extension module fails at COMPILE time (v1-locked)
If static resolution of any `import` / `from ... import ...` statement in the
compiled program (transitively, over the whole closure) resolves a name to a module
provided only as a CPython C extension (an `ExtensionFileLoader`-shipped binary,
including accelerator submodules such as `_json` under `json`, or `_socket` under
`socket` when not native-provided), the compiler MUST stop with a compile-time
error. There is no runtime fallback, no lazy "only errors if you actually run it",
and no partial success: `antipythany build` exits nonzero and emits no binary.
This is the v1 policy from master.md ("C extensions v1: None"); v2 lives in
Plan 10 and never relaxes this clause without a PR (§0 relaxation rule).

**Verify:** for each of a fixed probe set — `_json`, `_ssl`, `pydantic_core`
(third-party Rust extension), `numpy` — compile a one-line `import <name>` program
and assert: exit code != 0, no output artifact produced, and the stderr diagnostic
matches the format of C-08/C-10. Also assert transitive detection: a program that
`import json`s while `_json` is not native-provided fails at compile time naming
`_json` (and citing the importing module's file:line).

### C-08 — The diagnostic names the module and points to the extension policy
Every C-07 diagnostic MUST contain, verbatim: (1) the **full module name** that
resolved to an extension (e.g. `_socket`, not "a C module"); (2) the literal phrase
`C extension` (or the stable token `c-extension`); (3) a pointer to the extension
policy — the path `docs/compat-contract.md` clause `C-07` and the v2 plan location
`plans/antipythany/10-c-extensions-stretch/orchestrator.md` — so a user hitting the
error is routed to the rebuild-required v2 story instead of guessing.

**Verify:** compile `import _socket`; assert stderr matches the regex
`(?s).*_socket.*C extension.*(compat-contract|10-c-extensions).*` and that the
diagnostic includes the source location (`file:line:col`) of the import statement.

### C-09 — No silent stubs, no fake modules
The implementation MUST NOT substitute a pure-Python stub, a no-op module, or a
partially-implemented object for an extension module: `sys.modules` after a failed
import attempt is irrelevant because compilation never succeeds for that closure,
and the runtime MUST NOT register any object under an extension module's name in
the v1 registry. Detectable lie = bug.

**Verify:** grep-level + behavioral: build succeeds on programs that never import
extensions; for each extension probe name, there is no way to obtain a truthy
`sys.modules['<name>']` or `importlib.util.find_spec('<name>')` result that lies
about it being a no-op stub — assert `find_spec` either raises the same C-07 class
of error at build time (statically) or the module simply is never importable at
runtime.

### C-10 — Diagnostics are machine-stable
Each compile-time failure class has a stable diagnostic code token in the message,
so tests assert on the token instead of prose: `APX-CEXT` (C-07/C-08 extension
refusal), `APX-NOV1` (not in v1 allow-list, C-06), `APX-NOTFOUND` (module does not
exist). The token, module name, and `file:line:col` MUST appear even as the human
prose is reworded. Prose changes MUST not break tests that match only these fields.

**Verify:** for one example of each class, assert stderr contains the exact token
(`APX-CEXT` / `APX-NOV1` / `APX-NOTFOUND`), the offending module name, and a
`file:line:col` pointing at the import statement.

## 4. Integer semantics

### C-11 — Arbitrary precision, mathematically exact results
`int` is arbitrary precision with no maximum value other than available memory.
`+`, `-`, `*`, `//`, `%`, `**` (when the result is computable within memory),
`<<`, `>>`, `&`, `|`, `^`, comparisons, and hashing MUST produce the mathematically
exact result for the operand values. Tagged small-int representations (02.01) are
an internal optimization; crossing the tag boundary MUST be invisible.

**Verify:** corpus of exact-value checks matched against the reference, e.g.
`assert 2**1000 == 107150860718626732094842504906000181056140481170553360744375038835272459195351628820396420003426148648968321245281408000041496435453088469523307256781225520429611609149682955318989706311134631890555027011955885990765298747062624728376787651804452654953796059937637292629828016889939990801913101073137881881362870643791329714309607032318113675805864674454772459992416833577366633422348029643341829539584298758349879398785248112168566758390830259447272374565339072217068910894670390079296154967484181485853716709757909921025859772223894873212624814972712713378639828233866576869936140410759157843214988510129060878306756125321026523547164308127754647726395034699893019112634936233122628597272627588712643642488860015895292687039102008885003296509368280492779555396866751481896272771503013713081842237994513609875985415094267802039515185697333759507813707666083572541124086430264683881389855550279128248448`
and `(10**400 + 1) % 7`, and the int-float boundary: `2**53 + 1 == 2**53 + 1`
(`True`) while `float(2**53) + 1 == float(2**53)` (`True`) — byte-diffed via
ap-conform against the reference.

### C-12 — No overflow, wraparound, or saturation at representation boundaries
There is MUST NOT exist any integer operation on representable inputs that returns a
wrapped, saturated, or off-by-representation result (this includes the small-int tag
boundary and any i64 fast path). If a computation cannot complete within available
memory, the behavior is `MemoryError` or the harness limit — never a wrong number.

**Verify:** boundary sweep around the tag width `B` (the test agent reads `B` from a
documented constant in spec 05, not from code): for `k in (-2, -1, 0, 1)` around
`2**B`, assert `(2**B + k) + 1 == 2**B + k + 1` under antipythany and equals the
reference for the same expression; multiply/shift sweeps
`[(2**B - i) * (2**B + i) for i in range(4)]` byte-diff against the reference. Any
mismatch is a bug, not a divergence (no §7 id covers it).

### C-13 — `OverflowError` only where the reference raises it
The ONLY integer operations that raise `OverflowError` are the ones the reference
raises `OverflowError` for, with matching messages: e.g. converting an int too
large for the digit limit to `str` (`ValueError: Exceeds the limit ...` when
`sys.set_int_max_str_digits` applies), `float(10**400)`
(`OverflowError: int too large to convert to float`), and shift counts beyond
CPython's internal limits (`1 << (2**62)` → `OverflowError: too many shifts`).
`2**64`, `2**10000`, and chained arithmetic MUST NOT raise.

**Verify:** table-driven: each expression from the clause body run under both
implementations; assert identical exception type and identical last traceback line
(`OverflowError: <msg>` text) or identical value. Assert `print(2**10000)` succeeds
and its 3001-ish-digit output bytes match the reference.

## 5. Destruction timing (documented divergence area)

### C-14 — GC-based reclamation; no observable reference counting
Object lifetime is managed by the tracing GC (MMTk). Reference counts are not an
observable concept: there MUST NOT exist a pure-Python program whose stdout/stderr
or exit code can distinguish refcount-zero deterministic collection from
GC-liveness. `sys.getrefcount`/`sys.gettotalrefcount` are NOT part of the v1
surface (they belong to the excluded set of C-06 unless spec 05 lists them;
default: excluded).

**Verify:** two assertions. (1) `import sys; sys.getrefcount` raises
`AttributeError` under antipythany (absent from the v1 `sys` surface unless spec 05
lists it — divergence D3), while the reference returns an int — this program is
therefore excluded from the byte-diff corpus and registered under D3, not run as a
conformance case. (2) A program that creates an object with `__del__` inside a
function, returns, then `print("after")`: antipythany output is accepted as either
`bye\nafter` or `after\nbye` (timing-free), but after an explicit `gc.collect()`
before the `print`, output MUST be `bye\nafter` deterministically.

### C-15 — `__del__` timing is NOT guaranteed (documented divergence, §7-D1)
antipythany MUST run `__del__` for a collected object exactly when the collector
finalizes it, which is NOT necessarily when the last reference disappears.
A program MUST NOT be considered conformant-breaking if its `__del__` side effects
appear later than they would under CPython, at a `gc.collect()`, or at shutdown —
provided each object's `__del__` still runs (C-17) and never runs more than once
before process exit. This is a deliberate, documented divergence from the reference.

**Verify:** (positive) `class C: __del__: print("f", id(self))`; create 3 instances,
drop them, call `gc.collect()`, assert exactly 3 `f` lines appear before program
end. (divergence) a loop of `C()` temporaries with a `print("tick")` heartbeat
MAY interleave finalizers across ticks under antipythany where the reference runs
them inline; the conformance case for this pattern is written as a
properties-style test (set of finalizer lines at exit), not a byte-interleave diff.

### C-16 — Tests MUST NOT rely on finalization timing
Corpus and integration tests MUST NOT assert that a finalizer, weakref callback, or
`contextlib.aclosing`-style cleanup ran at a specific point determined by
reference-drop timing. Concretely forbidden: asserting `__del__` output ordering
relative to a following `print` without an intervening explicit `gc.collect()`;
asserting resource-close happened at `del obj`; counting `__del__` calls "by
now". Allowed: ordering after explicit `gc.collect()`, "at most once" and
"at least once by exit" properties, and context-manager `__exit__` (deterministic,
not a finalizer).

**Verify:** a lint over `tests/corpus/`: no case cited to a §7-D1 divergence that
lacks `gc.collect()` or is not marked property-style; the test harness rejects a
case that flaps (different pass/fail across two consecutive identical runs)
because it depends on GC timing.

### C-17 — Finalizers run at most once and before memory is reused
For every object, `__del__` (and any CPython-registered finalization hook) runs
zero or one times over the process lifetime, and MUST NOT run again after an
explicitly resurrected-and-re-dropped cycle beyond what CPython does; finalizer
state (`tp_del`-style "already finalized" bookkeeping) MUST be per-object, not
per-address, so a GC-moving heap (MMTk moves objects — 02.04) cannot re-fire a
finalizer because a slot was reused.

**Verify:** allocate/free cycle stress: 10k iterations creating a
`class Z: __del__ -> increments a counter`, dropping it, forcing `gc.collect()`,
and allocating other garbage; assert final-count == number-of-survivors-dropped
per C-15 rules AND no object's `__del__` fired twice (count keyed by a monotonic
creation token, not `id()`).

### C-18 — Weakref callbacks follow the same timing rules as finalizers
`weakref.ref(obj)` returns `None` only after collection (never too early), and
callbacks fire at collection time with C-15 timing (not necessarily at
reference-drop). `weakref.finalize` registration/invocation semantics match the
reference except for timing. `weakref` callbacks MUST NOT fire for objects that are
still reachable (no false collection).

**Verify:** hold a strong ref to `obj` in a list, register a callback, run 5 forced
`gc.collect()` cycles, assert callback has NOT fired and `ref() is obj`; drop the
strong ref, `gc.collect()`, assert `ref() is None` and callback fired once.

## 6. Float repr byte-exactness

### C-19 — `repr(float)` / `str(float)` are byte-exact vs the reference (v1-locked)
`repr` (and therefore `str`, print, f-string default format, and dict/list/tuple
repr of contained floats) of any finite, non-negative/ negative, normalized,
subnormal or integral float value MUST equal the reference's shortest
round-trip algorithm's output, byte for byte, including: exponent form
threshold (`1e+16` vs `1000000000000000.0`), trailing `.0` on integral values,
digit count, sign placement, and subnormal rendering.

**Verify:** property test over generated doubles (random bits + boundary set:
`0.0, -0.0, 5e-324, 1e-16, 1e15, 1e16, 1e17, 123456789012345.0, 1/3, 2/3,
9007199254740993.0`→`9007199254740992.0` precision collapse, etc.):
for each value `v` in the set and 10k random bit patterns, assert
`repr(v)` bytes equal reference bytes. ap-conform case:
`print(repr(1e16), repr(1e15), repr(-0.0), repr(5e-324))` byte-diffed.

### C-20 — Special values render identically to the reference
`repr`/`str` of `inf`, `-inf`, `nan` (including sign-propagating and payload
floats produced by the documented arithmetic of the reference) MUST be exactly
`inf`, `-inf`, `nan` as the reference renders them; arithmetic that produces them
(`1.0/0.0` → `ZeroDivisionError` message per reference, `float('inf') - float('inf')`
→ `nan`) MUST match the reference's values and messages.

**Verify:** assert the strings for `float('inf'), float('-inf'), float('nan'),
float('1e999')` and the `ZeroDivisionError: float division`/
`float modulo` message texts match the reference byte-for-byte via corpus case.

### C-21 — Round-trip exactness
For every float value `v`, `float(repr(v)) == v` (exact bit round-trip), and
`repr(float(s)) == s` for every `s` that is the reference's own repr (repr
stability / idempotence). The shortest-round-trip selection MUST prefer the same
digit string the reference does when multiple shortest candidates exist.

**Verify:** for the generated-bit corpus of C-19, assert
`float(repr(v)) == v or (v != v and float(repr(v)) != float(repr(v)))` (nan
handling) and `repr(float(repr(v))) == repr(v)`.

### C-22 — `float.__format__` matches the reference byte-exactly (delegated)
Explicit formatting (`format(v, spec)`, `%.`-op, f-string format specs) is in the
spec pack's scope (01-values-and-objects.md, `repr/str`); this contract asserts the
compatibility requirement at the same byte-exact bar: the outputs MUST be
byte-identical to the reference for every `(value, spec)` pair in the formatting
corpus. Clause is testable via the same ap-conform diff; spec 01 owns the spec-grid.

**Verify:** formatting matrix case: values from the C-19 set x specs
`['', 'e', '.6f', '>12.4g', '#x', 'n', '010.2%']` printed in a table, byte-diffed
against the reference; mismatch is a bug (no divergence id covers formatting).

## 7. Exit / stderr / stdout contract

### C-23 — `sys.exit` semantics match the reference
- `sys.exit()` / `sys.exit(None)` / falling off the end → process exit code `0`.
- `sys.exit(n)` for int `n` → process exit status `n & 0xFF` on POSIX (exactly as
  the reference yields on the same platform; `sys.exit(2)` → 2,
  `sys.exit(-1)` → 255).
- `sys.exit(x)` for non-int `x` → `str(x)` printed to **stderr** as the sole line
  and exit code `1` (reference semantics: message, not traceback).
- `SystemExit` raised inside a thread records the exception and the thread dies;
  only the main thread's `SystemExit` sets the process exit code (reference
  behavior).

**Verify:** one corpus case per bullet; ap-conform asserts both the exit code and
stdout/stderr bytes. `sys.exit(3)` → exit 3, empty stderr. `sys.exit("boom")` →
exit 1, stderr `boom\n`, stdout empty.

### C-24 — Uncaught exceptions: exit code 1, reference-format traceback on stderr
A script terminating via an uncaught exception MUST exit with code `1` and MUST
write the traceback to **stderr only**, in the CPython 3.14 format:
`Traceback (most recent call last):`, frame lines
`  File "<path>", line <n>, in <name>`, the source-line echo (imported modules
echo source text from the compiled-in source; `__main__` echoes from the file),
then `ExceptionType: message`. Chained exceptions use the reference's
`During handling of the above exception, another exception occurred:` /
`The above exception was the direct cause of the following exception:` separators.
The traceback text format is owned by spec 02; this clause binds exit code and
stream placement.

**Verify:** corpus cases: `raise ValueError("x")` uncaught → exit 1, stdout
`b""`, stderr matching the format regex from spec 02 and equal to reference bytes;
`assert False` → exit 1 with `AssertionError` tail (and reference behavior of
python `-O` NOT being default). Exception chaining case byte-diffed.

### C-25 — Interpreter-level errors match the reference exit codes
`KeyboardInterrupt`/`SIGINT` at top level → exit code `130` on POSIX (reference:
re-raises the signal); a `SyntaxError` reported at compile time for AoT/`build`
MUST fail the *compiler* with nonzero exit and a `file:line:col` diagnostic
(C-10 format family); a `SyntaxError` reached at runtime inside an `exec`/`eval`
of the supported v1 surface behaves per spec 02 (runtime exception, exit 1).

**Verify:** `build` a file containing `def f(:` → compiler exit != 0, diagnostic
names `SyntaxError` and location. Eval-level: `exec("1+")` inside an otherwise
valid program → traceback to stderr, exit 1, bytes match reference.

### C-26 — Stream bytes are byte-exact (encoding, buffering-visible order, newline)
stdout and stderr are captured per-process and compared as raw bytes; the runtime
MUST emit UTF-8 (matching the reference's default under the harness's pinned
environment), use the reference's newline translation, and — under the harness —
the per-stream content MUST match in order. Cross-stream interleaving is NOT
compared (ap-conform diffs streams separately, per C-28), which removes the
buffering race from the contract.

**Verify:** print non-ASCII (`"é漢\U0001F600"`), write to both streams, and confirm
stdout bytes == reference stdout bytes and stderr bytes == reference stderr bytes
under the pinned locale environment (`LC_ALL` / `PYTHONIOENCODING` documented in
the harness config); `print("a"); sys.stderr.write("b"); print("c")` → stdout
`a\nc\n` exactly.

### C-27 — Exit code 0 is reserved for clean exit
MUST NOT exit 0 when any exception traceback was printed; MUST NOT exit nonzero
when the program completed without an uncaught exception or explicit non-zero
`sys.exit`. A finalizer (`__del__`) raising at shutdown follows the reference:
message printed to stderr, process exit code becomes `120` (CPython's
"error in sys.delete" convention) — pinned by ap-conform case.

**Verify:** `atexit.register(lambda: 1/0)` style case and `__del__`-raises case,
each byte-diffed including exit code (reference: exit code 120 for unhandled
exception during interpreter finalization of that kind; if the reference's exact
code differs in any probe, spec 02/06 records the probed value — the contract
requirement is "equal to the reference", measured not assumed).

### C-28 — What ap-conform diffs is the whole observable contract
The conformance gate for every v1 behavior is: for each corpus script,
`stdout`, `stderr` (each as a byte string) and `exit code` produced under
antipythany MUST equal those produced under the C-01 reference. No other channel is
conformant-tested (timing, heap addresses, GC pauses, and `id()` values are
explicitly non-observable for conformance purposes).

**Verify:** the harness contract itself is tested (00.04 gate): self-comparison
`--impl python3.14` is green; injecting a deliberate byte difference in a fixture
impl flips its table row to FAIL with machine-readable output.

## 8. Implementation transparency

### C-29 — Tier-1 / JIT / specialization is invisible except by timing
Adaptive execution (02.05 inline caches, OSR, deopt; AoT vs JIT differences) MUST
NOT change stdout bytes, stderr bytes, or exit code of any conforming program.
Forced tier-0, adaptive mode, and AoT binary modes MUST agree. Deopt MUST restore
exact tier-0 mid-state — no duplicated or dropped side effects across an OSR or
deopt boundary. This is the "invisible-except-timing" rule cited by test leaf T-08.

**Verify:** run the full corpus in three modes (`--tier 0`, adaptive, built-binary)
via ap-conform; assert per-case identical results across modes; a loop
`sum of print-producing calls over 1M iterations` with OSR pressure MUST print the
reference sequence exactly once each.

### C-30 — Native and wasm artifacts are semantically identical (v1: wasm has no tier-1)
For the v1 corpus, native mode and wasm mode (`ap-host run`) MUST produce
byte-identical stdout, stderr, exit code — per the Plan 03 semantics-invariance
rule; divergences are fixed in shared layers (ap-obj/ap-ir), never by special-casing
wasm codegen. The known gap (wasm lacks tier-1 in v1) is timing-only and permitted
by C-29's carve-out.

**Verify:** T-12 parity runs: ap-conform over the full corpus with
`--impl python3.14`, `--impl 'antipythany run'`, `--impl 'ap-host run <file>.wasm'`;
all three columns equal per case for every corpus row.

## 9. Documented divergences register (§ per C-02)

| Id | Divergence | Clauses | Test policy |
| --- | --- | --- | --- |
| D1 | `__del__`/finalizer/weakref-callback timing is GC-based, not deterministic | C-14..C-18 | Property-style assertions only (C-16); never byte-interleave diffs of finalizer output against heartbeat prints |
| D2 | v1 refuses C-extension imports at compile time instead of loading them | C-06..C-10 | Positive test: refusal with stable diagnostic; no partial-module state observable |
| D3 | Refcount-introspection APIs (`sys.getrefcount`) absent unless spec 05 lists them | C-14 | Assert `AttributeError` at runtime |
| D4 | wasm mode has no tier-1 in v1 (timing only) | C-29, C-30 | Excluded from corpus by construction (no perf assertions in conformance cases) |

No other divergence exists. Anything not in D1–D4 MUST be byte-identical to the
reference (C-01, C-02, C-28).

## 10. Traceability (for 00.05 spec pack and the test tree)

- Spec pack sections consume this contract as follows: spec 01 (values) ← C-11..C-22;
  spec 02 (control-flow/exceptions) ← C-24, C-25; spec 04 (imports) ← C-04..C-10;
  spec 05 (builtins/stdlib list) ← C-05, C-06; spec 06 (observable behavior) ←
  C-23..C-30. Test leaves: T-01 ← C-07..C-10 + C-25; T-05 ← C-11..C-13, C-19..C-22;
  T-07 ← C-14..C-18 (D1); T-08 ← C-29; T-09 ← C-01..C-03, C-23..C-28; T-11 ←
  C-04..C-06; T-12 ← C-30.
- Every clause is stated so that a test agent holding only this file, `master.md`'s
  contracts section, and `docs/spec/` can write a passing-or-failing assertion
  without consulting implementation code.
