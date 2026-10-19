# ANTIPYTHANY Compatibility Contract (v1)

Leaf: 00.03 (`plans/antipythany/00-foundation-contract/orchestrator.md`).
Status: **normative**. This document is the compatibility contract between
antipythany and its reference implementation. It is an input to the spec pack
(00.05) and one of the two documents test agents read together with `docs/spec/`
(see `plans/antipythany/tests/README.md`). Where the spec pack elaborates a rule
stated here, this file wins on conflicts until corrected by PR (§11).

Relaxation rule (locked): Plan 10 / v2 research leaves may relax a clause marked
`v1-locked` ONLY by a PR that amends this file, with clause-id reasoning. No code
may relax a clause silently.

## Amendment register

| A-id | Date | Clause | What changed | Evidence (live probe, CPython 3.14.7) |
| --- | --- | --- | --- | --- |
| A-1 | 2026-10-16 | C-16 | Quoted ZeroDivisionError messages corrected: `1.0/0.0` and `1.0%0.0` raise `division by zero` (identical to the int text), not `float division` / `float modulo`. Rule (IEEE-754, message must equal reference) unchanged. | `python3.14 -c "1.0/0.0"` / `-c "1.0%0.0"` → `ZeroDivisionError: division by zero` |
| A-2 | 2026-10-16 | C-14 | Digit-limit text split into the two measured forms: str(int) form has no digit count; int(str) form adds `: value has N digits`. Both now quoted in full. | `python3.14 -c "str(10**5000)"` vs `-c "int('1'*5000)"` — see clause body for exact bytes |

## 0. How to read this document

- **MUST / MUST NOT / SHOULD / MAY** carry RFC-2119 force. A `MUST` violated in
  any mode (tier-0, tier-1 JIT, AoT binary, wasm) is a conformance bug.
- Every clause `C-nn` has a **Verify** line: a concrete assertion a test agent
  can write from this document plus the spec pack alone, without reading
  implementation code. Where a clause is about the *absence* of behavior, the
  Verify line says how to observe the absence.
- "The reference" means CPython 3.14.0 as pinned in C-01. Where this document
  gives example expected outputs, they are reference outputs obtained by running
  the snippet on CPython 3.14 (patch 3.14.7 during authoring; per C-01 the
  normative pin is 3.14.0 — values recorded from a non-pinned 3.14 patch are
  provisional until re-verified on 3.14.0). The spec pack restates them with
  full generality.
- Clause areas: C-01..C-03 reference implementation; C-04..C-06 supported
  subset; C-07..C-11 fail-loud extension rule; C-12..C-14 integer semantics;
  C-15..C-18 float semantics; C-19..C-23 destruction timing; C-24..C-27
  observability contract; C-28..C-31 exit codes and streams; C-32..C-33
  contract versioning. Divergence register: §9. Clause index: §12.

## 1. Reference implementation

### C-01 — Reference is CPython 3.14.0, pinned (v1-locked)
The reference implementation for all differential conformance is **CPython
3.14.0, exact patch version**. Byte-exact conformance is defined only against
this build. The harness invokes it as `python3.14` from `PATH` (00.04); that
binary MUST be a 3.14.0 release build (not a prerelease, not 3.14.x for x != 0).
If the exact build is unavailable, conformance runs MUST fail with a harness
error rather than silently comparing against a different version. A development
convenience override (environment variable documented in the harness, e.g.
`APX_CONFORM_ALLOW_PATCH_MISMATCH=1`) MAY permit another 3.14 patch for local
iteration, but any run so labeled is non-normative: expectations recorded from
it are provisional and MUST NOT be cited as reference bytes in this file, the
spec pack, or corpus headers.

**Verify:** run `python3.14 -VV`; assert `sys.version_info == (3, 14, 0,
'final', 0)` for the pinned build. Assert `ap-conform run` refuses to run
(exits nonzero with a version-mismatch diagnostic) when the probe reports any
other version, and that a mismatch-override run is visibly labeled non-normative
in its output.

### C-02 — Tie-break rule for undefined or ambiguous behavior
Where antipythany behavior is not fixed by this contract or the spec pack, the
reference's observed behavior on the pinned 3.14.0 build is normative by
default. A divergence from the reference is a bug UNLESS it is (a) listed as a
documented divergence in §9 of this file, or (b) a timing-only difference
explicitly sanctioned by a §9 entry. Any new divergence MUST be added here by PR
before it may ship.

**Verify:** for any corpus case, `assert antipythany_output == cpython_output`
holds byte-wise on all three channels (C-27) unless the case header cites a
divergence clause id. A CI check scans corpus case headers: every cited
divergence id exists in §9.

### C-03 — One Python version at a time
v1 supports exactly one Python language version: 3.14 (master.md, locked
decision). The compiler MUST reject source that the pinned ruff parser
(`PythonVersion::PY314`) rejects, with a byte-precise parse diagnostic (01.01),
and MUST NOT accept future-version-only syntax. The runtime MUST report
`sys.version_info == (3, 14, 0, 'final', 0)` identically to the reference.

**Verify:** (1) a program using 3.15-or-later-only syntax (when such grammar
exists to probe) fails to compile with a parse diagnostic naming the line and
column; (2) `python -c "import sys; print(sys.version_info)"` under antipythany
prints `sys.version_info(major=3, minor=14, micro=0, releaselevel='final',
serial=0)` byte-identically to the reference; (3) the parser rejects CPython
2-style `print "x"`.

## 2. Supported subset, v1

### C-04 — Pure-Python user programs are supported
Any user program written in the pure-Python subset of the 3.14 language (grammar
+ semantics covered by spec pack sections 01–04), whose complete transitive
static import closure satisfies C-05, MUST compile and MUST conform to the
reference on stdout, stderr, and exit code (C-27). "Pure Python" here means:
every module in the closure executes with no compiled-extension dependency.

**Verify:** each corpus slice case (values, control-flow, classes, exceptions,
generators, stdlib-v1) is by construction pure-Python and MUST pass ap-conform.
Conversely, a program containing no extension import MUST NOT be refused by the
compiler (no false rejections): assert compile success on the stdlib-v1 import
test set of C-05.

### C-05 — v1 stdlib = stdlib modules whose import closure needs no C extension
A standard-library module is in the v1 supported set iff its transitive import
closure (static `import`/`from` analysis at compile time) contains only (a)
pure-Python stdlib modules and (b) modules the antipythany runtime provides
natively as part of the v1 allow-list — including `builtins`, `sys`, `_thread`,
and the internals they pull in. Pure-Python stdlib modules that CPython itself
falls back to when their `_`-prefixed accelerator is missing (e.g. `functools`
without `_functools`) MAY be in v1 natively implemented; the accelerator modules
themselves are never visible to user code. The exact allow-list and the
native-provided list are enumerated in spec pack section 05
(`docs/spec/05-builtins-stdlib-v1.md`); this clause fixes the *rule*, spec 05
fixes the *list*. If spec 05 and this rule ever disagree, spec 05 is corrected
by PR.

**Named in-v1 floor.** Independently of spec 05's full list, the following
modules are known-in for v1 and MUST import and behave per the reference:
`sys`, `os` (os-lite subset per spec 05), `os.path`, `math`, `json`, `re`,
`itertools`, `functools`, `collections`, `collections.abc`, `dataclasses`,
`typing`, `contextlib`, `abc`, `enum`, `pathlib`, `textwrap`, `random`, `time`,
`string`, `io`, `traceback`. Removing any named module from v1 requires a PR
against this file (§11), not a spec-05 edit alone.

**Verify:** take the module list from spec 05. For each listed module `m`:
compiling `import m; print(m.__name__)` succeeds and output matches the
reference. For each of the 22 named floor modules: same, plus at least one
behavior case per module in the stdlib-v1 corpus slice. For each stdlib module
NOT listed and known extension-only (e.g. `_socket`, `_ssl`, `_json`,
`_sqlite3`): compiling `import <name>` fails per C-07.

### C-06 — Support is an allow-list, not a negative inference
v1 support decisions are made only by the positive allow-list (C-05 / spec 05).
The compiler MUST NOT accept an import merely because "it is pure Python and we
did not think about it": a module not present in the v1 registry produces the
same compile-time failure class as an extension module (C-07), with the
diagnostic naming the reason (`not-in-v1-stdlib` vs `c-extension`). Third-party
packages are out of scope for Plans 00–02; they enter the closure check only
once the packaging plan (04.01, uv integration) lands, and they are subject to
the same extension rule.

**Verify:** compile `import statistics` and `import urllib.robotparser`
(pure-Python in-v1 candidates) — both accepted once spec 05 lists them; compile
`import quixotic` (nonexistent) and a stdlib module deliberately excluded from
spec 05 — both rejected at compile time with diagnostics distinguishing
`module-not-found` from `not-in-v1-stdlib` from `c-extension` (assert the reason
token appears).

## 3. Fail-loud rule for C extensions

### C-07 — Any import resolving to a C-extension module fails loudly (v1-locked)
If static resolution of any `import` / `from ... import ...` statement in the
compiled program (transitively, over the whole closure) resolves a name to a
module provided only as a CPython C extension (an `ExtensionFileLoader`-shipped
binary, including accelerator submodules such as `_json` under `json`, or
`_socket` under `socket`, when not native-provided), the compiler MUST stop with
a compile-time error. There is no runtime fallback, no lazy "only errors if you
actually run it", and no partial success: `antipythany build` exits nonzero and
emits no binary. Imports whose target cannot be resolved statically (dynamic
`__import__` / `importlib.import_module` with a computed name) MUST fail at
**import time** with the same diagnostic content (C-08, C-10) instead of
succeeding silently. This is the v1 policy from master.md ("C extensions v1:
None"); v2 lives in Plan 10 and never relaxes this clause without a PR (§0
relaxation rule).

**Verify:** for each of a fixed probe set — `_json`, `_ssl`, `pydantic_core`
(third-party Rust extension), `numpy` — compile a one-line `import <name>`
program and assert: exit code != 0, no output artifact produced, and the stderr
diagnostic matches the format of C-08/C-10. Also assert transitive detection: a
program that `import json`s while `_json` is not native-provided fails at
compile time naming `_json` (and citing the importing module's file:line).
Finally, a program that computes a module name at runtime and imports an
extension name MUST raise an import-time error whose text satisfies the C-08
regex — never a successful import of a stub.

### C-08 — The diagnostic names the module and points to the extension policy
Every C-07 diagnostic MUST contain, verbatim: (1) the **full module name** that
resolved to an extension (e.g. `_socket`, not "a C module"); (2) the literal
phrase `C extension` (or the stable token `c-extension`); (3) a pointer to the
extension policy — the path `docs/compat-contract.md` clause `C-07` and the v2
plan location `plans/antipythany/10-c-extensions-stretch/orchestrator.md` — so a
user hitting the error is routed to the rebuild-required v2 story instead of
guessing.

**Verify:** compile `import _socket`; assert stderr matches the regex
`(?s).*_socket.*C extension.*(compat-contract|10-c-extensions).*` and that the
diagnostic includes the source location (`file:line:col`) of the import
statement.

### C-09 — No silent stubs, no fake modules
The implementation MUST NOT substitute a pure-Python stub, a no-op module, or a
partially-implemented object for an extension module: compilation never
succeeds for such a closure, and the runtime MUST NOT register any object under
an extension module's name in the v1 registry. Detectable lie = bug.

**Verify:** build succeeds on programs that never import extensions; for each
extension probe name, there is no way to obtain a truthy
`sys.modules['<name>']` or a lying `importlib.util.find_spec('<name>')` result —
assert `find_spec` either raises the same C-07 class of error at build time
(statically) or the module is simply never importable at runtime.

### C-10 — Diagnostics are machine-stable
Each compile-time failure class has a stable diagnostic code token in the
message, so tests assert on the token instead of prose: `APX-CEXT` (C-07/C-08
extension refusal), `APX-NOV1` (not in v1 allow-list, C-06), `APX-NOTFOUND`
(module does not exist). The token, module name, and `file:line:col` MUST appear
even as the human prose is reworded. Prose changes MUST NOT break tests that
match only these fields.

**Verify:** for one example of each class, assert stderr contains the exact
token (`APX-CEXT` / `APX-NOV1` / `APX-NOTFOUND`), the offending module name, and
a `file:line:col` pointing at the import statement.

### C-11 — Refusal is total per closure
An extension refusal rejects the entire compilation unit: the compiler MUST NOT
emit an artifact that "works until it reaches the import", and MUST NOT split a
program into a runnable part plus a refused part. The only delivered outcomes
for a closure containing an extension import are: compile-time refusal (static
case) or import-time error (dynamic case, C-07) — nothing in between.

**Verify:** for a program with an extension import guarded by `if False:` /
behind an untaken branch, compilation still fails with `APX-CEXT` (static
resolution is control-flow-insensitive); assert no binary is produced and the
diagnostic names the module.

## 4. Integer semantics

### C-12 — Arbitrary precision, mathematically exact results
`int` is arbitrary precision with no maximum value other than available memory.
`+`, `-`, `*`, `//`, `%`, `**` (when the result is computable within memory),
`<<`, `>>`, `&`, `|`, `^`, `divmod`, comparisons, and hashing MUST produce the
mathematically exact result for the operand values, including the sign rules of
floor division and modulo: `a // b` is floor division and `a % b` has the sign
of `b` (e.g. `-7 // 2 == -4`, `-7 % 2 == 1`, `7 // -2 == -4`, `7 % -2 == -1`),
identical to the reference. Tagged small-int representations (02.01) are an
internal optimization; crossing the tag boundary MUST be invisible.

**Verify:** corpus of exact-value checks matched against the reference, e.g.
`assert 2**1000 == <the exact 302-digit integer>` (the test pins the literal
from a reference run), `(10**400 + 1) % 7`, the four sign-rule identities above,
and the int-float boundary: `2**53 + 1 == 2**53 + 1` (`True`) while
`float(2**53) + 1 == float(2**53)` (`True`) — byte-diffed via ap-conform against
the reference.

### C-13 — No overflow, wraparound, or saturation at representation boundaries
There MUST NOT exist any integer operation on representable inputs that returns
a wrapped, saturated, or off-by-representation result (this includes the
small-int tag boundary and any i64 fast path). If a computation cannot complete
within available memory, the behavior is `MemoryError` or the harness limit —
never a wrong number.

**Verify:** boundary sweep around the tag width `B` (the test agent reads `B`
from a documented constant in spec 05, not from code): for `k in (-2, -1, 0,
1)` around `2**B`, assert `(2**B + k) + 1 == 2**B + k + 1` under antipythany and
equals the reference for the same expression; multiply/shift sweeps
`[(2**B - i) * (2**B + i) for i in range(4)]` byte-diff against the reference.
Any mismatch is a bug, not a divergence (no §9 id covers it).

### C-14 — Overflow/memory exceptions only where the reference raises them
The integer operations that raise are exactly those where the reference raises,
with matching type and message. On CPython 3.14 (verified live):
`float(10**400)` → `OverflowError: int too large to convert to float`; str-
converting an int that exceeds the int→str digit limit raises (unless the limit
is raised via `sys.set_int_max_str_digits`) — measured on 3.14.7, the two
conversion directions have **distinct message forms**, both normative
byte-exact (Amendment A-2):
- str(int) form (no digit count): `ValueError: Exceeds the limit (4300 digits)
  for integer string conversion; use sys.set_int_max_str_digits() to increase
  the limit`
- int(str) form (with digit count): `ValueError: Exceeds the limit (4300
  digits) for integer string conversion: value has 5000 digits; use
  sys.set_int_max_str_digits() to increase the limit` (N is the actual digit
  count of the input string); `print(1 <<
(2**62))` → `MemoryError` (allocation for the shift result), NOT
`OverflowError`. `2**64`, `2**10000`, and chained arithmetic on those values
MUST NOT raise.

**Verify:** table-driven: each expression from the clause body run under both
implementations; assert identical exception type and identical last traceback
line (`OverflowError: <msg>` / `MemoryError` text) or identical value bytes.
Assert `print(2**10000)` succeeds and its 3011-digit output bytes match the
reference.

## 5. Float semantics

### C-15 — `repr(float)` / `str(float)` are byte-exact vs the reference (v1-locked)
`repr` (and therefore `str`, `print`, f-string default format, and
dict/list/tuple repr of contained floats) of any finite float value — positive,
negative, normalized, subnormal, or integral — MUST equal the reference's
shortest round-trip repr, byte for byte, including: the exponent-form threshold
(`1e+16` vs `1000000000000000.0`), trailing `.0` on integral values, digit
count, sign placement (`-0.0` keeps the sign), and subnormal rendering
(`5e-324`). Reference bytes (verified on CPython 3.14):
`repr(1e16) == '1e+16'`, `repr(1e15) == '1000000000000000.0'`,
`repr(-0.0) == '-0.0'`, `repr(5e-324) == '5e-324'`, `repr(1/3) ==
'0.3333333333333333'`, `repr(10/3) == '3.3333333333333335'`.

**Verify:** property test over generated doubles (random bit patterns +
boundary set: `0.0, -0.0, 5e-324, 1e-16, 1e15, 1e16, 1e17, 123456789012345.0,
1/3, 2/3, float.fromhex('0x1.0p+53')`): for each value `v`, assert `repr(v)`
bytes equal reference bytes. ap-conform case:
`print(repr(1e16), repr(1e15), repr(-0.0), repr(5e-324))` byte-diffed.

### C-16 — Arithmetic is IEEE-754 double; special values render as the reference
All `float` arithmetic (`+ - * / // % **`, `math` functions over floats) is
IEEE-754 binary64 with the reference's rounding and special-value behavior.
`repr`/`str` of `inf`, `-inf`, `nan` are exactly `inf`, `-inf`, `nan`.
`1.0 / 0.0` raises `ZeroDivisionError: division by zero` — verified live on
3.14.7; `1.0 % 0.0` likewise (Amendment A-1: earlier text quoted the pre-3.14
messages `float division` / `float modulo`, which are NOT what the reference
produces — the float and int operand forms share the identical message,
matching spec 01 S-01-8). `float('inf') - float('inf')` → `nan`; `float('1e999')`
→ `inf`; `-0.0 + 0.0` → `0.0` and `-0.0 * -1` → `0.0` per IEEE rules as the
reference implements them. `sys.float_repr_style` MUST be `'short'`.

**Verify:** assert the strings for `float('inf'), float('-inf'), float('nan'),
float('1e999')` and the `ZeroDivisionError: division by zero` message text
(both `1.0/0.0` and `1.0%0.0`) match the reference byte-for-byte via corpus
case; assert `math.copysign(1, -0.0) == -1.0` to pin sign-of-zero fidelity.

### C-17 — Round-trip exactness and repr idempotence
For every float value `v` (except nan), `float(repr(v)) == v` (exact bit
round-trip), and `repr(float(repr(v))) == repr(v)`. The shortest-round-trip
selection MUST prefer the same digit string the reference does when multiple
shortest candidates exist (David Gay / Grisu-style shortest repr, matching
CPython's `float_repr_style == 'short'` algorithm output).

**Verify:** for the generated-bit corpus of C-15, assert
`float(repr(v)) == v or (v != v)` (nan handling) and
`repr(float(repr(v))) == repr(v)`.

### C-18 — Float formatting matches the reference byte-exactly
Explicit formatting (`format(v, spec)`, `%`-formatting, f-string format specs)
meets the same byte-exact bar: outputs MUST be byte-identical to the reference
for every `(value, spec)` pair in the formatting corpus. Spec 01 owns the spec
grid; this clause binds the compatibility requirement.

**Verify:** formatting matrix case: values from the C-15 set × specs
`['', 'e', '.6f', '>12.4g', 'n', '010.2%', '.0f']` printed in a table,
byte-diffed against the reference; mismatch is a bug (no divergence id covers
formatting).

## 6. Destruction timing (documented divergence area)

### C-19 — GC-based reclamation; no observable reference counting
Object lifetime is managed by the tracing GC (MMTk). Reference counts are not
an observable concept: there MUST NOT exist a pure-Python program whose
stdout/stderr or exit code can distinguish refcount-zero deterministic
collection from GC-liveness. Refcount-introspection APIs (`sys.getrefcount`,
`sys.gettotalrefcount`) are **corpus-prohibited** (spec 05: stub-safe values
allowed, never in a conformance case); a corpus case MUST NOT call them, and
their presence, absence, or return values are never gated.

**Verify:** a lint over `tests/corpus/` asserts no case references
`getrefcount`/`gettotalrefcount`/`getallocatedblocks`; the harness rejects such
a case at load time.

### C-20 — `__del__` timing is NOT guaranteed (documented divergence D1)
antipythany MUST run `__del__` for a collected object exactly when the
collector finalizes it, which is NOT necessarily when the last reference
disappears. A program is NOT considered non-conformant if its `__del__` side
effects appear later than they would under CPython — at a `gc.collect()`, or at
shutdown — provided each object's `__del__` still runs (C-21) and never runs
more than once. Canonical example of a program whose output MAY differ:

```python
class Noisy:
    def __init__(self): print("make")
    def __del__(self): print("del")

for _ in range(3):
    Noisy()
print("loop done")
```

CPython 3.14 prints `make\ndel\n` interleaved three times, then `loop done`;
antipythany MAY print all three `make` lines and defer some or all `del` lines
until after `loop done` (or until shutdown). After an explicit `gc.collect()`
before the final `print`, the `del` lines MUST all have appeared (finalization
is forced by collection, so `make×3, del×3, loop done` becomes deterministic).

**Verify:** (positive) create 3 instances, drop the references, call
`gc.collect()`, assert exactly 3 `del` lines appear before program end.
(divergence, D1) the interleaved case above is written as a property-style test
(set of finalizer lines present at exit, each exactly once), never a
byte-interleave diff.

### C-21 — Finalizers run at most once, and run eventually
For every object, `__del__` runs zero or one times over the process lifetime,
and every object whose finalizer is registered and which becomes unreachable
MUST have its `__del__` run by process exit (eventually-run guarantee).
Finalizer "already finalized" bookkeeping MUST be per-object, not
per-address, so a GC-moving heap (MMTk moves objects — 02.04) cannot re-fire a
finalizer because a slot was reused. An exception raised inside `__del__` is
unraisable: it is reported to stderr in the reference's
`Exception ignored while calling deallocator ...` form (spec 06 owns the exact
text) and MUST NOT crash the process or change the exit code (C-29).

**Verify:** stress: 10k iterations creating a `class Z: __del__ -> increments
a counter` instance, dropping it, forcing `gc.collect()`, interleaved with
other garbage; assert final count == number of dropped instances AND no
object's `__del__` fired twice (count keyed by a monotonic creation token, not
`id()`). Unraisable case: a `__del__` that raises produces the reference's
stderr shape and exit code 0.

### C-22 — Weakref callbacks follow the same timing rules as finalizers
`weakref.ref(obj)()` returns non-`None` while the object is reachable (never a
false collection), and `None` only after collection. Callbacks fire at
collection time with C-20 timing (not necessarily at reference-drop).
`weakref.finalize` registration/invocation semantics match the reference except
for timing; `weakref.finalize`'s atexit path still runs. Callbacks MUST NOT
fire for objects that are still reachable.

**Verify:** hold a strong ref to `obj` in a list, register a callback, run 5
forced `gc.collect()` cycles, assert the callback has NOT fired and
`ref() is obj`; drop the strong ref, `gc.collect()`, assert `ref() is None` and
the callback fired exactly once.

### C-23 — Tests MUST NOT rely on finalization timing
Corpus and integration tests MUST NOT assert that a finalizer, weakref
callback, or generator-`close` style cleanup ran at a specific point determined
by reference-drop timing. Concretely forbidden: asserting `__del__` output
ordering relative to a following `print` without an intervening explicit
`gc.collect()`; asserting resource-close happened at `del obj`; counting
`__del__` calls "by now". Allowed: ordering after explicit `gc.collect()`,
"at most once" and "at least once by exit" properties, and context-manager
`__exit__` (deterministic, not a finalizer).

**Verify:** a lint over `tests/corpus/`: no case cited to divergence D1 lacks
`gc.collect()` or is not marked property-style; the test harness rejects a case
that flaps (different pass/fail across two consecutive identical runs) because
it depends on GC timing.

## 7. Observability contract

### C-24 — The comparison channels are exactly three
Programs are compared on **stdout bytes**, **stderr bytes**, and **exit code**
— each captured as a separate stream, compared independently, byte-exact, no
normalization. No other channel is conformance-tested: timing, heap addresses,
GC pauses, `id()` values, object `repr` addresses (`<object at 0x...>` forms are
compared only with the address masked per spec 06), and wall-clock values are
explicitly non-observable for conformance purposes.

**Verify:** the harness contract itself is tested (00.04 gate): self-comparison
`--impl python3.14` is green; injecting a deliberate byte difference in a
fixture impl flips its table row to FAIL with machine-readable output; merged
`2>&1` invocation is refused by the harness.

### C-25 — Traceback text matches CPython 3.14 format
A script terminating via an uncaught exception writes the traceback to **stderr
only**, in the CPython 3.14 format: `Traceback (most recent call last):`;
frame lines `  File "<path>", line <n>, in <name>`; the source-line echo with
the reference's `^^^^` anchor marks where applicable; then
`ExceptionType: message` as the last line. Chained exceptions use the
reference's exact connectors: `During handling of the above exception, another
exception occurred:` and `The above exception was the direct cause of the
following exception:`. The traceback text format is owned by spec 02/06; this
clause binds the format requirement and the stream placement.

**Verify:** corpus case: `raise ValueError("x")` uncaught → exit 1, stdout
`b""`, stderr byte-equal to reference stderr. Exception-chaining case
(`raise X from Y`, implicit chaining) byte-diffed including the connector
lines.

### C-26 — Repr ordering: stable vs unstable outputs
**Stable (byte-anchored, MUST match the reference exactly):** dict iteration
and repr order — dicts are insertion-ordered by language spec (3.7+) and in v1;
`list`/`tuple` order; `str`/`bytes` content; `repr` of dicts, lists, tuples,
and insertion-ordered views derived from them. A corpus case MAY byte-diff
`print({'a': 1, 'b': 2})` against the reference.
**Unstable (MUST NOT be byte-anchored):** `set` and `frozenset` iteration and
repr order — unspecified by the language and hash-randomization-dependent in
CPython (order shifts with `PYTHONHASHSEED`; verified). Corpus cases MUST
compare sets via `sorted(s)` or membership, never via `print(s)` byte-diff —
except the documented narrow exception of small-int sets where CPython's
`hash(int) == int` makes order stable across seeds (SHOULD still be avoided;
spec 06 S-06-17 governs). `PYTHONHASHSEED` MUST NOT need to be honored by v1:
any conforming program produces identical output with the variable unset.

**Verify:** (1) corpus case `print({'b': 1, 'a': 2})` byte-diffed — insertion
order `b, a` preserved. (2) lint: no corpus case prints a bare `set`/`frozenset`
in a byte-diff without `sorted()` or the int-set exception marker. (3) a
program printing `sorted({'c','a','b'})` matches the reference under both
implementations regardless of hash seed.

## 8. Exit codes, `sys.exit`, and streams

### C-27 — What ap-conform diffs is the whole observable contract
The conformance gate for every v1 behavior is: for each corpus script,
`stdout`, `stderr` (each as a byte string) and `exit code` produced under
antipythany MUST equal those produced under the C-01 reference. Stream bytes
are byte-exact: UTF-8 output matching the reference under the harness's pinned
environment, reference newline translation, per-stream program order.
Cross-stream interleaving is NOT compared (streams are captured separately),
which removes the buffering race from the contract.

**Verify:** print non-ASCII (`"é漢\U0001F600"`) to both streams and confirm
per-stream bytes == reference under the pinned locale environment;
`print("a"); sys.stderr.write("b"); print("c")` → stdout `a\nc\n` exactly,
stderr `b`, with no cross-stream order assertion.

### C-28 — `sys.exit` semantics match the reference
- `sys.exit()` / `sys.exit(None)` / falling off the end → exit code `0`.
- `sys.exit(n)` for int `n` → process exit status `n & 0xFF` on POSIX exactly
  as the reference yields on the same platform (`sys.exit(2)` → 2,
  `sys.exit(-1)` → 255 — verified).
- `sys.exit(x)` for non-int `x` → `str(x)` written to **stderr** followed by a
  newline as the sole output, exit code `1` (reference semantics: message, not
  traceback; verified with a `__str__`-defining object that the raw `str()` is
  used, and `[1]` → `[1]`, `0.0` → `0.0`).
- `SystemExit` raised inside a non-main thread ends that thread without setting
  the process exit code; only the main thread's `SystemExit` sets the process
  exit code (reference behavior).

**Verify:** one corpus case per bullet; ap-conform asserts both the exit code
and stdout/stderr bytes. `sys.exit(3)` → exit 3, empty stderr. `sys.exit("boom")`
→ exit 1, stderr `boom\n`, stdout empty.

### C-29 — Uncaught exceptions, KeyboardInterrupt, and finalization exit codes
- Uncaught exception: exit code `1`, traceback on stderr only (C-25), stdout
  untouched.
- `KeyboardInterrupt` / SIGINT at top level: reference behavior — the
  `KeyboardInterrupt` traceback is printed to stderr and the process exit code
  is `130` on POSIX (verified: CPython 3.14 re-raises the signal after
  printing).
- Exceptions raised in a `__del__`, weakref callback, or `atexit` callback are
  unraisable: reported to stderr in the reference's `Exception ignored in/at
  ...` form and the exit code is **unchanged** (verified: a `__del__`-raises
  program and an `atexit`-raises program both exit `0` on the reference; the
  contract requirement is "equal to the reference, measured not assumed" — if a
  probe ever shows a different code, spec 06 records it).
- A `SyntaxError` in the compiled file fails the **compiler** (build verb):
  nonzero exit, `file:line:col` diagnostic (C-10 format family), no artifact. A
  `SyntaxError` raised at runtime inside `exec`/`eval` is an ordinary
  exception: traceback per C-25, exit 1.

**Verify:** corpus cases: `raise ValueError("x")` → exit 1; `assert False` →
exit 1 with `AssertionError` tail; `atexit.register(lambda: 1/0)` → exit 0 with
the `Exception ignored in atexit callback` stderr shape; `build` a file
containing `def f(:` → compiler exit != 0; `exec("1+")` inside an otherwise
valid program → exit 1, bytes match reference.

### C-30 — Exit code 0 is reserved for clean exit
MUST NOT exit 0 when an uncaught-exception traceback was printed (C-29); MUST
NOT exit nonzero when the program completed without an uncaught exception or an
explicit nonzero `sys.exit` — including when finalizers, weakref callbacks, or
atexit callbacks ran and printed unraisable-exception reports (C-29 third
bullet).

**Verify:** for each C-29 case, assert the exit code equals the reference's
measured code (not an assumed constant); a CI probe re-measures the reference
values so a CPython patch change is caught.

### C-31 — Execution tiers and artifacts are behaviorally invisible
Adaptive execution (02.05 inline caches, OSR, deopt) and AoT-vs-JIT differences
MUST NOT change stdout bytes, stderr bytes, or exit code of any conforming
program. Forced tier-0, adaptive, and AoT binary modes MUST agree on every
corpus case. Native and wasm artifacts MUST produce byte-identical
stdout/stderr/exit code per the Plan 03 semantics-invariance rule (wasm's lack
of tier-1 in v1 is timing-only and permitted). Divergences are fixed in shared
layers (ap-obj/ap-ir), never by special-casing wasm codegen.

**Verify:** run the full corpus in three modes (`--tier 0`, adaptive,
built binary) and, additionally, native-vs-wasm parity runs via ap-host; assert
per-case identical results across all modes; a 1M-iteration loop with OSR
pressure MUST print the reference sequence exactly once each (no duplicated or
dropped side effects across an OSR/deopt boundary).

## 9. Documented divergences register

| Id | Divergence | Clauses | Test policy |
| --- | --- | --- | --- |
| D1 | `__del__`/finalizer/weakref-callback timing is GC-based, not deterministic at last-reference-drop | C-19..C-23 | Property-style assertions only (C-23); never byte-interleave diffs of finalizer output against heartbeat prints |
| D2 | v1 refuses C-extension imports at compile time instead of loading them | C-06..C-11 | Positive test: refusal with stable diagnostic; no partial-module state observable |
| D3 | Refcount-introspection APIs are corpus-prohibited (presence/absence not gated) | C-19 | Lint + harness rejection; no conformance case may call them |
| D4 | wasm mode has no tier-1 in v1 (timing only) | C-31 | Excluded from corpus by construction (no perf assertions in conformance cases) |
| D5 | `re` matching aborts with `re.error("ap: matching step limit exceeded")` after 4M atomic steps; CPython's backtracker instead runs unboundedly (catastrophic patterns like `(a+)+b` on `"a"*22` complete in CPython, raise here) | C-01, C-02 | No corpus case may rely on completing catastrophic-backtracking patterns; the abort is a stable diagnostic |

No other divergence exists. Anything not in D1–D5 MUST be byte-identical to the
reference on the three channels (C-01, C-02, C-24, C-27).

## 10. Contract versioning and evolution

### C-32 — The contract changes only by PR; clause ids are immutable
This file evolves exclusively by pull request against the repository; no other
document (including the spec pack or an implementation leaf) may override a
clause. **Clause ids are never reused**: a clause that is deleted or superseded
has its id RETIRED — the id stays in the §12 index marked `RETIRED (see C-nn)`
and MUST NOT be reassigned to new content; new clauses take the next unused
number. Renumbering existing clauses is forbidden for the same reason: corpus
headers, spec-pack text, and plan leaves cite ids. Editorial rewording that
does not change normative force MUST be flagged `non-normative edit` in the PR.

**Verify:** a CI check on this file: (1) every clause id in the body appears in
the §12 index and vice versa; (2) no id in the index changed its one-line
summary relative to the previous commit unless the PR declares it; (3) ids
marked RETIRED never reappear as live clause headers.

### C-33 — Corpus monotonicity (master rule)
"Corpus only grows. Never weaken a test to pass a gate" (master.md; AGENTS.md
execution protocol). A contract amendment MUST NOT be used to justify deleting
or weakening an existing corpus case: if a clause change makes an existing case
fail, the PR must either (a) revert the clause change, (b) re-scope the case to
the divergence register with a new D-id and keep the case as a property test,
or (c) replace the case with a strictly stronger one. Corpus cases are removed
only when they are unrunnable by construction (e.g. they called a now-
corpus-prohibited API) and the PR shows their gate coverage moved to a new or
remaining case.

**Verify:** a CI check counts corpus cases per divergence/clause citation over
history: per-case counts are monotonically non-decreasing except in PRs whose
description itemizes the moved coverage; `git log --diff-filter=D --
tests/corpus/` since the last release is empty or each deletion links such a
PR.

## 11. Relationship to the spec pack

Spec pack sections consume this contract as follows: spec 01 (values) ←
C-12..C-18; spec 02 (control-flow/exceptions) ← C-25, C-29; spec 04 (imports)
← C-04..C-11; spec 05 (builtins/stdlib list) ← C-05, C-06, C-13; spec 06
(observable behavior) ← C-24..C-30. Test leaves (per `plans/antipythany/tests/`):
T-01 ← C-07..C-11 + C-29; T-05 ← C-12..C-18; T-07 ← C-19..C-23 (D1); T-08 ←
C-31; T-09 ← C-01..C-03, C-27..C-30; T-11 ← C-04..C-06; T-12 ← C-31. Where a
spec-pack S-clause elaborates a C-clause, the C-clause is authoritative on
conflict until this file is amended (C-32). Every clause is stated so that a
test agent holding only this file, `master.md`'s contracts section, and
`docs/spec/` can write a passing-or-failing assertion without consulting
implementation code.

## 12. Clause index

| Id | Summary (one line) | Section |
| --- | --- | --- |
| C-01 | Reference is CPython 3.14.0, exact patch, pinned (v1-locked) | 1 |
| C-02 | Unspecified behavior defers to the reference; divergences only via §9 | 1 |
| C-03 | Exactly one language version: 3.14; future syntax rejected; version_info fixed | 1 |
| C-04 | Pure-Python programs in the v1 closure MUST compile and conform | 2 |
| C-05 | v1 stdlib rule: no-C-extension closure + named 22-module floor | 2 |
| C-06 | Support is a positive allow-list; unlisted = refused with reason | 2 |
| C-07 | C-extension imports fail at compile time (dynamic: import time), totally | 3 |
| C-08 | Extension diagnostic names module + points to extension policy | 3 |
| C-09 | No silent stubs or fake modules for extensions | 3 |
| C-10 | Stable diagnostic tokens: APX-CEXT / APX-NOV1 / APX-NOTFOUND | 3 |
| C-11 | Refusal is total per closure; no partial artifacts | 3 |
| C-12 | Ints arbitrary precision; exact floor-division/modulo sign rules | 4 |
| C-13 | No wraparound/saturation at representation boundaries | 4 |
| C-14 | Overflow/memory exceptions exactly where the reference raises them | 4 |
| C-15 | Float repr/str byte-exact, shortest round-trip (v1-locked) | 5 |
| C-16 | IEEE-754 binary64 arithmetic; inf/nan rendering and messages | 5 |
| C-17 | Round-trip exactness and repr idempotence | 5 |
| C-18 | Float format specs byte-exact vs reference | 5 |
| C-19 | Refcounting non-observable; refcount APIs corpus-prohibited | 6 |
| C-20 | `__del__` timing not guaranteed (divergence D1), with canonical example | 6 |
| C-21 | Finalizers run at most once and eventually; unraisable exceptions | 6 |
| C-22 | Weakref callbacks: same timing rules, no false collections | 6 |
| C-23 | Tests must not depend on finalization timing | 6 |
| C-24 | Comparison channels: stdout bytes, stderr bytes, exit code — nothing else | 7 |
| C-25 | Traceback text format + chaining connectors match reference; stderr only | 7 |
| C-26 | Repr ordering: dicts stable (byte-anchored), sets unstable (sorted only) | 7 |
| C-27 | Byte-exact streams (UTF-8, newline, per-stream order); no cross-stream diff | 8 |
| C-28 | `sys.exit` int/non-int/None/thread semantics match reference | 8 |
| C-29 | Uncaught exceptions (1), SIGINT (130), unraisable-at-exit (rc unchanged) | 8 |
| C-30 | Exit code 0 reserved for clean exit only | 8 |
| C-31 | Tier-1/JIT/AoT/wasm differences behaviorally invisible | 8 |
| C-32 | Contract changes by PR only; clause ids never reused or renumbered | 10 |
| C-33 | Corpus monotonicity: never delete/weaken a case to pass a gate | 10 |
