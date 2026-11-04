# Spec 06 — Observable Behavior (what ap-conform diffs)

Part of the spec pack (00.05). Source of truth for test agents (T-09 binary
conformance gates 02.06; T-12 wasm parity cites the byte-identical rule; T-13
host/WASI cites this doc). Derived from `plans/antipythony/master.md` (interface
contract rule 3: "run script under CPython and antipythony, diff bytes on
stdout/stderr and exit code"), 00.03 compat contract, and CPython **3.14.7**
behavior verified live on `/opt/homebrew/bin/python3.14`. Clause ids `S-06-NN`.

All "verified" markers below mean: the exact bytes/rc were produced by running the
snippet on python3.14 3.14.7 in this repo's environment during spec authoring.

---

## 6.1 The conformance triad

**S-06-1** ap-conform MUST compare exactly three channels per corpus case: (1)
stdout as a byte string, (2) stderr as a byte string, (3) process exit code. No
channel normalization, no whitespace trimming, no line-ending munging, no trailing-
newline synthesis or removal — **byte-exact on each stream, compared independently**.
A case passes iff all three match CPython 3.14.0-reference byte-for-byte (pin note:
reference binary is whatever `python3.14` on PATH reports; 3.14.0 vs .7 drift is
governed by the compat contract's version-pin clause, not here).

**S-06-2** stdout and stderr MUST be captured as separate streams by the runner
(`subprocess` with separate pipes). The runner MUST NOT use `2>&1` merging.
Cross-stream interleaving is therefore never diffed (see S-06-13).

## 6.2 stdout rules

**S-06-3** `print(*args, sep=' ', end='\n', file=sys.stdout, flush=False)` renders
`str(arg)` for each arg joined by `sep`, then `end`. Default `sep` is a single
space `0x20`; default `end` is `0x0A`. `str(x) is x.__str__()`, falling back to
`repr(x)` per spec 01; `print()` with no args writes exactly `0x0A`. Verified:
`print(1, 2, 3)` → bytes `31 20 32 20 33 0A` (od -c: `1   2   3 \n`).

**S-06-4** No trailing-whitespace normalization ANYWHERE. Whitespace before `\n`,
spaces at end of line, tabs, `\r` written explicitly — all MUST appear byte-exact.
Verified: `print('trailing   ')` → `t r a i l i n g (3×space) \n` (od -c). A
formatter, f-string, or repr that produces trailing spaces MUST emit them.

**S-06-5** Encoding: stdout MUST be UTF-8 (8-bit superset, `errors='strict'`)
when not a TTY. Verified on 3.14.7 pipe: `sys.stdout.encoding == 'utf-8'`, and
`print('héllo→✓')` emitted bytes `68 c3 a9 6c 6c 6f e2 86 92 e2 9c 93 0a`.
`PYTHONIOENCODING` is OUT-of-v1 scope (S-06-15): corpus scripts run with a clean
env; v1 MUST NOT read it. Unicode chars beyond the source encoding are gated only
through UTF-8 bytes.

**S-06-6** Buffering for the diff contract: when stdout is a pipe it is
block-buffered (verified: `line_buffering=False`, `write_through=False` under pipe
on 3.14.7); when a TTY, line-buffered. This is OBSERVABLE ONLY via cross-stream
interleaving, which ap-conform never diffs (S-06-2); the per-stream final bytes are
identical in both modes because the runtime MUST flush stdout before process exit
on every termination path (normal, SystemExit, uncaught exception) except hard
`os._exit` / abort, which are corpus-prohibited (S-06-19). v1 MUST flush stdout
before exit on all three paths.

## 6.3 stderr rules — uncaught exception report

**S-06-7** An uncaught non-SystemExit exception MUST be reported by the default
`sys.excepthook` equivalent, on stderr, with this byte-exact structure (3.14.7
format, verified):

```
Traceback (most recent call last):
  File "<path>", line <n>, in <qualname>
    <stripped source line>
    <anchor line if applicable>
<ExcType>: <str(exc)>
```

- Header `Traceback (most recent call last):\n` then one block per frame,
  innermost LAST. Frame order: `<module>` outermost-first.
- `File` path: the script path as resolved by the runner (absolute, matching how
  CPython was invoked — ap-conform MUST invoke both impls with the identical argv
  so path bytes coincide).
- `in <qualname>`: `<module>` at top level; plain function qualname otherwise.
- Source line from `linecache`-equivalent, leading whitespace stripped of the
  common indent but PRESERVING relative indent, one leading 4-space pad (verified:
  `    1/0`). If the source line is unavailable → the frame prints only the
  `  File "..."` line (no fake line).
- **Anchor lines (3.14-specific, MUST match)**: fine-grained `~~^~~`/`^^^^` markers
  under the error span for statements/expressions. Verified examples on 3.14.7:
  `1/0` → `    1/0\n    ~^~`; `raise ValueError('E1')` (after `print('OUT1'); ` on
  same line) → anchor under the `raise` statement only:
  `    print('OUT1'); raise ValueError('E1')\n                   ^^^^^^^^^^^^^^^^^^^^^^`;
  call spans (`'a'+1` → `    ~~~^~`; `eval('1+'  )` → `    ~~~~^^^^^^^^`). Anchor
  computation depends on the 3.14 traceback column model — this is a Plan-01/02
  fidelity requirement; until proven, corpus anchors for S-06-7 pin single-line
  module-level statements AND the multi-line `def a(): b()` inline-def form shown
  in the anchors section (verified bytes reproduced).
- `<ExcType>`: `type(exc).__qualname__`-style full name WITHOUT module prefix for
  builtins; with module prefix (`pkg.mod.ExcName`) for user exceptions defined in
  an importable module. Verified: `ValueError: invalid literal for int() with base
  10: 'nonexistent'`; `ModuleNotFoundError: No module named 'nonexist_mod'`; a
  custom-class exception renders `mymod.MyError: ...` per spec-03 qualname rules.
- No args (`raise ValueError`) → the line is `ValueError` with NO trailing colon
  (verified: `ValueError` bare).
- `str(exc)` used, never `repr(exc)`; multi-arg exceptions join with `", "`
  (spec 01 `BaseException.__str__`). Verified: `raise ValueError(['a','b'])` →
  `ValueError: ['a', 'b']`; `raise ValueError(b'\xff')` →
  `ValueError: b'\xff'`.

**S-06-8** Chaining MUST render in CPython order and with the two exact connector
blocks (verified 3.14.7 byte-for-byte):

- `raise X from Y` → Y's report first, then:
  `\nThe above exception was the direct cause of the following exception:\n\n` then
  X's report.
- implicit context (exception raised while handling) → prior report, then
  `\nDuring handling of the above exception, another exception occurred:\n\n` then
  new report. Chains recurse (verified: 3-deep context chain rendered correctly).
- `from None` SUPPRESSES context: only the final report (verified).
- Recursion truncation: repeated frames collapse to
  `[Previous line repeated <n> more times]` (spec 02 clause; cite there).

**S-06-9** SyntaxError (compile-time, surfaced at run) uses the NO-Traceback-header
render (verified 3.14.7):

```
  File "<path>", line <n>
    <source line>
    <pointer caret>
SyntaxError: <message>
```

e.g. `x = (` → `  File "…/sx.py", line 1\n    x = (\n        ^\nSyntaxError: '(' was never closed`.
`eval('1+')` inside a running program renders as a normal traceback whose last
frame is the eval'd `<string>` block ending `SyntaxError: invalid syntax` (verified
two-part render). Exit code 1 (S-06-11).

**S-06-10** Warning lines (`warnings.warn`, default filters): stderr, format
verified 3.14.7:
`<file>:<lineno>: <Category>: <message>\n  <stripped source line>\n`.
Default action for `UserWarning` prints once per location. Full filter semantics
spec 02/05; the LINE FORMAT is a MUST here.

## 6.4 SystemExit handling

**S-06-11** The `sys.exit(x)` / `raise SystemExit(x)` contract (all verified
3.14.7; SystemExit is NEVER printed as a traceback, and its code channel differs
per payload type):

| payload | exit code | output |
| --- | --- | --- |
| `sys.exit(n)` int | `n & 0xFF` (Python-level masking via C `exit(3)`: 3→3, 300→44, 256→0, -1→255) | none |
| `sys.exit()` / `sys.exit(None)` | 0 | none |
| `sys.exit(s)` any non-int (str, list, dict, float) | 1 | `str(s)` if `s` is `str`, else `repr(s)`, written to **stderr**, then `\n` (verified: `'boom'`→stderr `boom\n`, stdout empty, rc 1; `[1]`→stderr `[1]`, rc 1; `{'a': 1}`→stderr `{'a': 1}\n`; `0.0`→stderr `0.0\n` rc 1). Empty string → a single `\n` on stderr (verified od: `0000000  \\n`). |
| `sys.exit(b)` `bool` | int channel (`True` is `int` subclass): rc = `b & 0xFF` (True→1, False→0), NO output (verified: `sys.exit(True)` rc 1 stderr empty; `sys.exit(False)` rc 0 stderr empty) | — |
| `sys.exit('42')` | 1 — NOT 42 (no numeric coercion; verified) | `42\n` on stderr |

- Caught-then-reraised SystemExit keeps its code (verified rc 5). `finally: return`
  inside a function swallows SystemExit → rc 0 (verified).
- `atexit` callbacks MUST still run on every SystemExit path, after main-body
  output flush and BEFORE the exit code is handed to the OS; LIFO order (verified:
  two registered `print`s → `B` then `A`, rc 0; and with `sys.exit(3)`: `bye`
  printed, rc stayed 3 — corpus anchor #5).
- int subclass semantics: `int` and `bool` take the code channel; `enum.IntEnum`
  members follow CPython's `__index__` conversion (corpus-prohibited beyond
  builtins).

## 6.5 Exit-code contract (full v1 matrix)

**S-06-12** Exit codes v1 MUST produce, each verified on 3.14.7:

| condition | code |
| --- | --- |
| normal completion, `SystemExit(None/0)`, bare `sys.exit()` | 0 |
| uncaught non-SystemExit exception (incl. `RecursionError`, `assert False`, `StopIteration`/`GeneratorExit` raised manually to top) | 1 |
| `SystemExit(n)` int | `n & 0xFF` |
| `SystemExit(<non-int>)` | 1 |
| `KeyboardInterrupt` reaching top level — including a plain `raise KeyboardInterrupt` (verified rc **130** both ways) and real SIGINT (CPython rc 130 = 128+SIGINT via self-`os.kill`; OS-signal delivery itself is gated outside the corpus — `signal` is OUT-of-v1) | 130 |
| CLI-level "file not found" (`python script` where script missing → rc 2, message `can't open file '<abs path>': [Errno 2] No such file or directory` prefixed by argv[0]-style runner name) | 2 |

v1 runner (`antipythony run/build` verbs, ap-conform `--impl`) MUST mirror row for
row. The CLI "can't open file" message's prefix is the impl's own argv[0]; ap-
conform invokes scripts as `impl script.py` so the prefix is NOT diffed — only the
rc 2 + suffix shape is (documented divergence: prefix text may name a different
launcher binary).

## 6.6 Buffering / ordering / interleaving PIN

**S-06-13** Observed CPython 3.14.7 rule for REDIRECTED PIPES (`2>&1` merging),
verified three ways:
1. `print('OUT1'); raise ValueError('E1')` piped 2>&1 → stderr traceback appears
   FIRST, buffered `OUT1\n` flushed at finalization LAST.
2. `print('OUT1', flush=True); raise ValueError('E1')` piped → `OUT1` first,
   traceback after.
3. `print('o'); sys.stderr.write('e\n')` piped → `e` then `o` (same mechanism:
   stderr is unbuffered/write-through; stdout flushes at exit).

PINNED RULE: stderr is unbuffered; stdout is block-buffered when not a TTY and is
flushed AFTER the exception report at finalization (NOT "stdout flushed before
stderr"). A program that explicitly flushes stdout (or `flush=True`) interleaves in
program order. v1 MUST reproduce this interleaving byte-for-byte for merged-pipe
use... **but**: ap-conform diffs the streams separately (S-06-2), so this rule is
MUST-honor only for the wasm-host/interactive UX and for T-12 parity of per-stream
bytes; corpus cases MUST NOT rely on cross-stream interleaving (S-06-17), since the
diff contract is stream-separated. `-u` (unbuffered) mode verified to change order;
it is a CLI flag, OUT-of-corpus.

**S-06-14** Ordering within a stream is program order, total. Nothing is
re-ordered, deduplicated, or batched differently: the final byte sequence of each
stream MUST equal CPython's, including partial trailing lines (a script whose last
stdout write lacks `\n` ends stdout without `\n`; `print` adds it; `sys.stdout.write`
does not). Verified: `sys.stdout.write('A'); flush; stderr.write('B\n');
stdout.write('C')` → stdout bytes `ABC` (C flushed at exit), stderr `B\n`.

## 6.7 Env vars and nondeterminism — v1 scope

**S-06-15** v1 honors NO Python-behavior env vars beyond OS-locale-independent
defaults. OUT-of-v1 (fail-loud is NOT the rule here — they are simply IGNORED, and
documented as such): `PYTHONHASHSEED`, `PYTHONIOENCODING`, `PYTHONUTF8` (v1 stdout
is always UTF-8 per S-06-5), `PYTHONWARNINGS`, `PYTHONOPTIMIZE`, `PYTHONDONTWRITE-
BYTECODE`, `PYTHONPATH` (build-time path handling belongs to Plan 04; runtime
`sys.path` seeding from env is v2), `PYTHONHOME`. ap-conform MUST launch BOTH
reference and impl with the same scrubbed environment (only `PATH`, `HOME`, `LANG`
pass-through) so neither impl's behavior depends on them; the scrub itself is
part of the harness contract, and CPython verified to emit `utf-8` encoding
regardless of `LANG=C` on this build (verified: `LANG=C` pipe → `utf-8`).

**S-06-16** **PYTHONHASHSEED is NOT honored by v1, and MUST NOT be needed**: dicts
are insertion-ordered in v1 (spec 01), so dict iteration order carries no hash
randomness. CPython-side: `PYTHONHASHSEED=0/1/2` verified to change `hash(str)`
and set iteration order (`'ccc','bb','a','d'` ordering shifted per seed). v1's
str hash is deterministic across runs (no seed concept); therefore:
- `hash(s)` for `s: str` values are NOT corpus-anchored (v1 makes no promise to
  equal CPython's seeded or unseeded values — documented divergence).
- `hash(int)` / `hash(None)` / `hash(True)` ARE deterministic in CPython
  (verified: `hash(123456789)` identical under seeds 0 and 7) and MAY be anchored
  for int-valued specs of spec 01.
- dict key ITERATION order = insertion order in both CPython (3.7+ language
  guarantee) and v1 → safe to anchor.

**S-06-17** **Corpus-prohibited timing/ordering-dependent behaviors** (a corpus
case containing any of these is invalid; the ban is normative for test authors):
1. `__del__` / `weakref.ref` callback / `gc` finalizer TIMING — tracing GC
   divergence is the compat contract's standing documented exception; scripts MUST
   NOT observe when finalizers run. Finalizer OUTPUT on the normal exit path is
   allowed only via `atexit` (S-06-11), never via `__del__`.
2. Set / `frozenset` / `dict`-keys (only via insertion order, safe) — SET
   iteration order specifically: unspecified by the language, hash-seed dependent
   in CPython (verified order shifts with `PYTHONHASHSEED`) → set iteration order
   is CORPUS-PROHIBITED (`print(list(s))` is an invalid anchor). Exception: small
   int sets where CPython's hash==value makes order stable (e.g. `{1,2,3}` →
   `[1, 2, 3]` verified across seeds) — still SHOULD be avoided; anchor `sorted(s)`
   instead.
3. `time.monotonic/perf_counter/time` VALUES, sleep precision, thread-scheduling
   interleavings (`threading` cases must synchronize explicitly; no
   "join-with-timeout implies sleep-based ordering"), signal delivery timing.
4. `os.getpid`, `id()` values, `sys.getrefcount/getsizeof/getallocatedblocks`,
   `hash(str)`, memory addresses in reprs (`<object at 0x...>` — never diff: the
   default `object.__repr__` embeds addresses and v1 MAY render them; CPython's
   address digits are nondeterministic → `repr()` of a plain object with default
   `__repr__` is corpus-PROHIBITED as an anchor; `dir(obj)` names only).
5. `input()` / interactive stdin reads (harness supplies empty stdin →
   `EOFError` is deterministic but still prohibited to keep the contract simple),
   TTY-dependent formatting (color codes: CPython 3.14 colorizes tracebacks when
   stderr is a TTY; ap-conform pipes streams so BOTH impls run non-TTY → plain
   bytes; v1 MUST NOT colorize on pipes).
6. `PYTHONHASHSEED`-sensitive: `random` module sequences (module is OUT-of-v1 per
   spec 05 anyway), dict-ordering exploits reading `hash()` of strs.
7. Multi-process behavior (`os.fork`, `multiprocessing`, `subprocess`) — all
   OUT-of-v1; prohibited by extension.

**S-06-18** Float formatting byte-exactness: `repr(float)` shortest-roundtrip,
`str(float)` == `repr(float)` in 3.x, `format` with explicit spec per spec 01.
Verified 3.14.7 renders: `0.1` → `0.1`; `1e300` → `1e+300`; `float('inf')` → `inf`;
`float('nan')` → `nan`; `-0.0` → `-0.0`; `1/3` → `0.3333333333333333`;
`repr(2.675)` → `2.675`; `repr(1e16)` → `1e+16`; `repr(1e-5)` → `1e-05`;
`format(0.1,'.2f')` → `0.10`; `round(2.675, 2)` → `2.67`; `round(0.5)`→`0`,
`round(1.5)`→`2` (banker's rounding — MUST match). These are normative anchors.

**S-06-19** `sys.stdout`/`sys.stderr` MUST exist at startup (fd-1/fd-2 backed),
`sys.argv[0]` = the script path as invoked (verified: `print(sys.argv[0])` under
`python -S i4.py extra1` → `i4.py`, relative as-given); trailing argv items in
`sys.argv[1:]` in order. Redirection to closed fds (`>&-`) behavior is
corpus-prohibited (CPython 3.14's own render is a platform detail).

**S-06-20** Broken-pipe hygiene: a program piping stdout into a closed pipe, on
clean exit, emits on stderr the finalization diagnostic
`Exception ignored while flushing sys.stdout:\nBrokenPipeError: [Errno 32] Broken pipe\n`
(verified 3.14.7) and dies by signal (rc from shell = 141, SIGPIPE). ap-conform
MUST NOT pipe stdout to consumers that close early: runner reads both pipes to EOF.
The 120-exit convention some apps use is app-level, not runtime-level; v1 runtime
MUST emit the "Exception ignored" diagnostic text byte-exactly on the same
condition.

## 6.8 WASM parity of this section

**S-06-21** The WASM artifact (via `ap-host`, WASI stdout/stderr) MUST produce the
identical stdout bytes, stderr bytes, and exit code for every corpus case
(T-12 "byte-identical rule"; cited by tests/README). WASI preopened-fd quirks
(closed-stdin → `input` EOFError shape) are prohibited anchors per S-06-17 #5.

---

## Corpus anchors (verified on CPython 3.14.7, `python3.14 -S`)

One-line scripts; stdout/stderr captured as SEPARATE streams per S-06-2 (`` = empty
stream). All outputs produced by live runs on python3.14 3.14.7 during authoring.

1. **S-06-3** `print(1, 2, 3)` → stdout `1 2 3\n`, stderr ``, rc 0.
2. **S-06-4** `print('trailing   ')` → stdout `trailing   \n` — three trailing
   spaces preserved verbatim (od -c verified), stderr ``, rc 0.
3. **S-06-11** `import sys; sys.exit(3)` → ``, ``, rc 3; `sys.exit(300)` → rc 44;
   `sys.exit(256)` → rc 0; `sys.exit(-1)` → rc 255; `sys.exit()`/`sys.exit(0)` →
   rc 0 — all with NO output on any stream.
4. **S-06-11** `import sys; sys.exit('boom')` → stdout ``, stderr `boom\n`, rc 1;
   `sys.exit('')` → stderr `\n`, rc 1; `sys.exit('42')` → rc 1 (NOT 42);
   `raise SystemExit({'a': 1})` → stderr `{'a': 1}\n`, rc 1; `sys.exit(True)` →
   int channel: rc 1, NO output; `sys.exit(0.0)` → stderr `0.0\n`, rc 1.
5. **S-06-11** `import sys, atexit; atexit.register(print, 'bye'); sys.exit(3)` →
   stdout `bye\n`, rc 3 (atexit runs on the SystemExit path; two registrations run
   LIFO: `B` then `A`).
6. **S-06-7/S-06-11 contrast** `raise RuntimeError('boom')` (line 1 of c6.py) →
   stderr `Traceback (most recent call last):\n  File "<abs>/c6.py", line 1, in <module>\n    raise RuntimeError('boom')\nRuntimeError: boom\n`, stdout ``, rc 1.
7. **S-06-7** `1/0` (`-c`) → stderr tail `    1/0\n    ~^~\nZeroDivisionError: division by zero\n` (3.14 anchor line), rc 1. And `x = int('nonexistent')` (line 1) → tail `    x = int('nonexistent')\nValueError: invalid literal for int() with base 10: 'nonexistent'\n` — NO anchor line for a plain assignment; rc 1.
8. **S-06-8** `try: 1/0\nexcept ZeroDivisionError as e: raise RuntimeError('outer') from e` → two traceback blocks joined by `\nThe above exception was the direct cause of the following exception:\n\n`, ending `RuntimeError: outer`; rc 1. `from None` variant → single block only (context suppressed).
9. **S-06-9** sx.py = `x = (` → stderr `  File "<abs>/sx.py", line 1\n    x = (\n        ^\nSyntaxError: '(' was never closed\n` (no `Traceback` header), rc 1.
10. **S-06-12** `raise KeyboardInterrupt` (plain builtin raise — no `signal`
    module needed, so it IS corpus-anchorable) on CPython 3.14.7 → stderr
    `Traceback (most recent call last):\n  File "<abs>/ki.py", line 1, in <module>\n    raise KeyboardInterrupt\nKeyboardInterrupt\n`, rc **130** (verified for both `-c` and file form).
    Real OS-SIGINT delivery (rc 130 = 128+SIGINT, verified via self-`os.kill`) is
    gated outside the corpus: v1 has no `signal` module (spec 05).
11. **S-06-6/S-06-14** `print('OUT1'); raise ValueError('E1')` → stdout `OUT1\n`
    AND stderr full traceback, rc 1 (separate-stream capture; interleaving not
    diffed). `sys.stdout.write('A'); sys.stdout.flush(); sys.stderr.write('B\n');
    sys.stdout.write('C')` → stdout `ABC` (no trailing `\n`), stderr `B\n`, rc 0.
12. **S-06-18/S-06-19** `print(0.1, 1e300, float('inf'), float('nan'), -0.0, 1/3)`
    → stdout `0.1 1e+300 inf nan -0.0 0.3333333333333333\n`, rc 0;
    `print(round(2.675, 2), round(0.5), round(1.5))` → `2.67 0 2\n` (banker's
    rounding); `print(sys.argv[0])` run as file i4.py → `i4.py\n`, rc 0.
