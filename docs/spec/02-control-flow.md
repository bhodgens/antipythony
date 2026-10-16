# Spec 02 — Control Flow

Clause prefix: `S-02-*`. Normative keywords are **MUST**, **SHOULD**, **UNSPECIFIED**.
Reference behavior: CPython 3.14 (every quoted output produced by `python3.14`
3.14.7). Written from master.md, the compatibility contract, and CPython semantics —
never from implementation code. The lowering rule from Plan 01 binds this doc: no
compilation/lowering strategy may reorder or drop operations with observable effects;
every construct below defines the observable order tests may rely on.

Convention for tracebacks: `ap-conform` runs each corpus case as a FILE; the
`File "..."` string in an uncaught-exception traceback is the path as passed on the
command line. Both implementations receive the identical argv, so path text is
byte-comparable. Exit codes for uncaught non-SystemExit exceptions are 1.

---

## 1. Name scoping (LEGB)

- **S-02-1 (MUST)** Name resolution order is Local → Enclosing function locals
  (statically, every lexically enclosing `def`/`lambda` frame) → Global (module) →
  Builtin. First binding wins at each level; lookup never "skips" an enclosing
  scope that has the name. Observed: nested `def` shadows resolve innermost:
  module `x="g"`, `f()` sets local `x="l"`, inner `i()` returns it → `f()` prints `l`,
  module still `g`.
- **S-02-2 (MUST)** A name assigned ANYWHERE in a function body (including in a
  branch that never executes) is local to that function for the ENTIRE body —
  determined statically, not dynamically. Reading it before assignment raises
  `UnboundLocalError` with exact text: `cannot access local variable 'z' where it is
  not associated with a value` (note: NOT "local variable ... referenced before
  assignment" — that is pre-3.11 text). `UnboundLocalError` is a subclass of
  `NameError`. The frontend verifier (01.04) MUST predict every corpus
  UnboundLocalError from this static rule with matching message text.
- **S-02-3 (MUST)** `global name` makes all uses of `name` in the function refer to
  the module global; `nonlocal name` binds to the nearest ENCLOSING function scope
  (never module scope). `global x` and `nonlocal x` for the same x in one function is
  a compile-time `SyntaxError: name 'x' is nonlocal and global`. A read-only closure
  reference needs no declaration (S-02-6).
  Observed: `nonlocal n` counter increments across calls → `1 2 3`.
- **S-02-4 (MUST)** `nonlocal` without an existing binding in an enclosing function
  scope fails at COMPILE time: `SyntaxError: no binding for nonlocal 'x' found`
  (caret under the statement). `nonlocal` at module level is likewise SyntaxError.
- **S-02-5 (MUST)** Class bodies do NOT form an enclosing scope for their methods:
  a method reading a class-level name raises `NameError: name 'x' is not defined`
  (observed: `class C: x=10; def m(self): return x` → NameError). Comprehensions and
  generator expressions have their OWN implicit scope: the iteration variable does not
  leak to the enclosing function/module (`[i for i in range(3)]` leaves outer `i`
  untouched; `j` is not in module namespace afterwards). Exception: a comprehension's
  FIRST iterable is evaluated in the enclosing scope. `for` loop variables DO leak
  into the enclosing function scope (`for k in range(3)` leaves `k == 2`).
- **S-02-6 (MUST)** `del x` unbinds a name: later use raises
  `NameError: name 'x' is not defined`; `del` of an unbound name raises
  `NameError: name 'y' is not defined`.

## 2. Functions and argument passing

- **S-02-7 (MUST)** A `def` statement, at execution time, evaluates (in this order):
  default-value expressions, decorators (innermost first), then binds the resulting
  function to the name in the enclosing scope. Default expressions are evaluated
  ONCE at def time, NOT per call: `def mk(seq=[]): seq.append(1)` → two calls print
  `[1, 1] [1, 1]` (shared mutable default). Decorators receive the function and their
  return value replaces it (observed order: default evaluated before decorator:
  `['d1', 'deco:g']`).
- **S-02-8 (MUST)** Parameter kinds and order: positional-only (`a, /`),
  positional-or-keyword, `*args` (collects remaining positionals into a tuple),
  keyword-only (after `*` or `*args`), `**kwargs` (collects remaining keywords into a
  dict). Observed: `def f(a, b=2, *args, c, d=4, **kw)` called
  `f(1, 3, 5, c=7, e=8)` returns `(1, 3, (5,), 7, 4, ['e'])`.
- **S-02-9 (MUST)** Call-site `f(*seq)` unpacks positionals left-to-right;
  `f(**map)` unpacks keyword entries; both allowed multiple times. Evaluation order
  of arguments at a call site is left-to-right after the callable object is obtained.
- **S-02-10 (MUST)** Binding errors raise `TypeError` with exact message text
  (function's `__qualname__` used as the name):
  - too few: `f() missing 1 required positional argument: 'b'`; multi-name:
    `missing 2 required positional arguments: 'a' and 'b'`;
  - too many: `f() takes 2 positional arguments but 3 were given`;
  - unknown keyword: `f() got an unexpected keyword argument 'c'`;
  - duplicate binding: `f() got multiple values for argument 'a'`;
  - missing keyword-only: `f() missing 1 required keyword-only argument: 'c'`;
  - keyword passed for a positional-only parameter: `f() got some positional-only
    arguments passed as keyword arguments: 'a'`.
  A duplicate keyword in the call LITERAL (`g(a=2, a=3)`) fails at compile time:
  `SyntaxError: keyword argument repeated: a`.
- **S-02-11 (MUST)** `return` with no expression returns `None`; `return` outside a
  function body is a SyntaxError. A `lambda` body is one expression whose value is
  returned implicitly; lambda's traceback frame name is `<lambda>` (S-02-38).

## 3. Closures and cells

- **S-02-12 (MUST)** An inner function referencing an enclosing variable captures the
  VARIABLE, not the value: late binding. Observed: `[lambda: i for i in range(3)]`
  called yields `[2, 2, 2]`. Rebinding the outer variable after closure creation is
  visible through the closure.
- **S-02-13 (MUST)** All closures over the same enclosing variable share ONE cell:
  a `nonlocal` write in one inner function is observed by every other closure over
  that variable. Writing requires `nonlocal` (S-02-3); reading does not.
- **S-02-14 (MUST)** Closure lifetime is independent of the defining frame's
  execution: a function returned from its defining scope keeps its cells alive and
  callable. (Under the tracing GC, no reference-cycle leak semantics apply — cells
  are collected when unreachable; destruction-order caveats are spec 06/contract.)

## 4. Control statements

- **S-02-15 (MUST)** `if/elif/else`: first true condition's body runs; conditions are
  tested with truthiness (spec 01 S-01-27/28/29); later branches not evaluated.
- **S-02-16 (MUST)** `while cond: body [else: eb]`: cond tested before each
  iteration; `else` runs iff the loop terminated by condition becoming false — NOT
  via `break`, and NOT skipped by exception propagation. `while True: ... break`
  skips `else`. A zero-iteration loop still runs `else`.
- **S-02-17 (MUST)** `for TARGET in ITERABLE:` obtains `iter(iterable)` ONCE, then
  calls `__next__` repeatedly, binding TARGET to each value (assignment-target
  unpacking semantics, S-02-19), until `StopIteration` — which is swallowed as normal
  loop termination and never leaks. Iterables accepted: any object with `__iter__`
  (list, tuple, str yields 1-char str, dict yields KEYS in insertion order, set,
  generators, file objects) or a legacy `__getitem__` sequence (v1 corpus: not
  relied upon). Non-iterable raises `TypeError: 'int' object is not iterable`.
  Mutating the underlying list during iteration is defined by the underlying
  iterator's live-index behavior; corpus SHOULD NOT rely on it.
- **S-02-18 (MUST)** `break` exits the innermost enclosing `for`/`while`, skipping
  its `else`. `continue` jumps to the next iteration test (for: next `__next__`).
  `break`/`continue` outside a loop are SyntaxError. `pass` does nothing.
  Observed break/else: a `for` that breaks prints nothing from its `else`; a
  completed `while` runs its `else` body (`while-else 2`).
- **S-02-19 (MUST)** Tuple/sequence unpacking `a, b = seq` and starred unpacking
  `a, *b, c = seq` (at most one star; `b` is a fresh LIST of the middle elements,
  empty when none: `first, *rest = [7]` → `rest == []`). Wrong arity raises
  `ValueError: not enough values to unpack (expected 2, got 1)` /
  `ValueError: too many values to unpack (expected 2, got 3)` — exact text.
  Chained assignment `x = y = z = 5` binds all names left-to-right to the SAME object.
  Augmented assignment `xs += [5]` on a list mutates in place (spec 01 S-01-19).

## 5. Generators

- **S-02-20 (MUST)** A function body containing `yield` compiles to a generator
  FUNCTION: calling it returns a generator object without executing any body code
  (lazy start). `iter(gen) is gen`.
- **S-02-21 (MUST)** `yield expr` suspends and produces `expr` to `__next__`/`send`;
  execution resumes when the generator is advanced again. The VALUE of the yield
  EXPRESSION is whatever `send(v)` passed (or `None` for `next()`/`send(None)`):
  `x = yield 1; ...` sees `x == 7` after `g.send(7)` (observed `got 7`).
  A bare `yield` yields `None`.
- **S-02-22 (MUST)** `StopIteration` propagation: a generator returning normally
  raises `StopIteration(value)` to its ADVANCER, where `StopIteration.value` is the
  generator's return value (`return x+1` after `send(10)` → `e.value == 11`;
  plain exhaustion → `e.value is None`). For-in loops and list()/tuple() consume it;
  uncaught in expression context it ends the consuming protocol.
- **S-02-23 (MUST)** PEP 479: a `StopIteration` raised INSIDE a generator body
  (not from its own `yield`) is transformed into `RuntimeError` with exact text
  `generator raised StopIteration`, chaining the original as `__cause__`. Observed:
  `yield next(iter([]))` inside `list(g())` → caught `RuntimeError`,
  `type(e).__cause__).__name__ == "StopIteration"`.
- **S-02-24 (MUST)** `g.send(v)` on a NOT-YET-STARTED generator with `v != None`
  raises `TypeError: can't send non-None value to a just-started generator`.
- **S-02-25 (MUST)** `g.close()`: throws `GeneratorExit` at the current yield point.
  Generator's `finally`/cleanup bodies RUN (observed `cleanup` printed on close).
  If the generator handles GeneratorExit and yields AGAIN, close() raises
  `RuntimeError: generator ignored GeneratorExit`. A generator that simply exits
  (via return or StopIteration) after GeneratorExit is fine; close() itself does NOT
  propagate GeneratorExit to the caller.
- **S-02-26 (MUST)** `yield from sub`: delegates iteration to `sub`'s iterator
  transparently (sends flow in, values flow out, yielded values surface unchanged —
  observed: `yield from inner()` relays `send(5)` making inner yield 15); when sub
  finishes, `yield from`'s value is sub's `StopIteration.value` (return value):
  `def g2(): yield from [1,2]; return 3` → advancing past end gives
  `StopIteration.value == 3`. Exceptions raised inside sub propagate through (traceback
  includes BOTH the delegating `~~~^^` frame and the sub frame, observed).

## 6. Exceptions

- **S-02-27 (MUST)** `raise EXPR` evaluates EXPR: if it is a subclass object of
  BaseException it is raised as-is; if it is a CLASS deriving from BaseException it is
  instantiated with no args first (`raise MyErr` → `MyErr()` with empty args,
  `repr` → `MyErr()`); anything else raises
  `TypeError: exceptions must derive from BaseException`.
- **S-02-28 (MUST)** Bare `raise` inside an except block re-raises the in-flight
  exception, PRESERVING its original traceback (the re-raise adds no new frame to the
  location where the exception was originally raised — observed traceback shows both
  the original `1 / 0` frame and the raise site frames; the original traceback entries
  come FIRST after the call chain, i.e. `raise e` style re-display rules: a bare
  `raise` keeps the SAME traceback object, so the printed frames list the original
  raise location). Bare `raise` with no in-flight exception raises
  `RuntimeError: No active exception to reraise`.
- **S-02-29 (MUST)** Exception MESSAGE rules (used by traceback last line and `str(e)`):
  for generic exceptions `str(e)` returns `str(args[0])` when exactly one arg was
  passed (`Exception("solo")` → `solo`, `ValueError(("t",2))` → `('t', 2)` since the
  tuple IS the one arg), `str(args)` (tuple rendering) when 0 or 2+ args —
  `ValueError(1,2)` → `(1, 2)`; zero args → `''`). **KeyError is the one built-in
  override**: with exactly one arg it returns `repr(args[0])` — `str(KeyError("solo"))`
  → `'solo'` (quoted) — and falls back to BaseException behavior for 0 or 2+ args
  (`str(KeyError(("t",2)))` → `('t', 2)`). `repr(e)` == `ClassName(<args tuple as
  repr>)`: `ValueError()` → `ValueError()`, `ValueError("v")` → `ValueError('v')`,
  `KeyError("k")` → `KeyError('k')`, two args → `ValueError(1, 2)`. Consequently the
  traceback summary line `KeyError: 'kk'` (observed) is str-of-the-exception, not a
  special traceback rule. Custom subclasses follow the generic rules (observed `E42`
  from `__init__` calling `super().__init__(f"E{code}")`).
- **S-02-30 (MUST)** `try/except` dispatch: clauses are tested IN SOURCE ORDER; the
  FIRST clause whose class (or any element of its tuple) matches via
  `isinstance(exc, clause)` (subclass-of-BaseException hierarchy, so `except
  LookupError` catches `KeyError` and `IndexError`; `except E` catches subclass `F`)
  handles it. A non-matching clause falls through; if none match, the exception
  propagates after `finally`/cleanup of enclosing blocks. `except CLAUSE as e:` binds
  e ONLY inside the handler — the name is deleted at handler exit: later reference
  raises `NameError: name 'e' is not defined` (prevents reference cycles).
  A non-class except target (e.g. `except 5:`) raises
  `TypeError: catching classes that do not inherit from BaseException is not allowed`
  (chained on top of the in-flight exception, observed full double traceback).
- **S-02-31 (MUST)** Bare `except:` (no expression) catches any `BaseException`
  subclass including `SystemExit`, `KeyboardInterrupt`, `GeneratorExit`. It runs only
  after all preceding clauses fail to match.
- **S-02-32 (MUST)** `try/else`: the `else` body runs iff the `try` body completed
  WITHOUT raising anything. If the try body raised and any except clause handled it,
  `else` is SKIPPED. Observed: clean try → `else ran`; caught try → else skipped.
  Exceptions raised in the `else` body propagate outward but are NOT caught by the
  same statement's clauses.
- **S-02-33 (MUST)** `finally` body runs on EVERY path leaving the try statement:
  normal completion, handled exception (after the handler), unhandled exception
  (before propagation), `break`/`continue`/`return`. If the `finally` body itself
  raises, the ORIGINAL pending exception is replaced by the new one and chained as
  `__context__` (implicit chaining, S-02-35).
  **Return-in-finally overrides an earlier return**: `try: return 1 / finally:
  return 2` → function returns 2; likewise a `return` in finally swallows a pending
  exception (the exception does NOT propagate). CPython 3.14 emits a
  SyntaxWarning at compile time for this pattern (observed stderr, two lines, no
  marker line):
  `<path>:<line>: SyntaxWarning: 'return' in a 'finally' block` then the source
  line indented 2 spaces (`  return 2`); antipythany MUST implement the override
  semantics and SHOULD emit the identical warning (warning text itself is NOT gated
  for byte-exactness in v1).
  A return/raise/break inside `try` defers until finally completes (observed: `F`
  printed before `zd`).
- **S-02-34 (MUST)** Implicit chaining: when a new exception is raised while another
  is IN FLIGHT (uncaught-in-handler, during cleanup, in finally), the new exception's
  `__context__` is the in-flight one and `__suppress_context__` is False. When no
  exception is in flight, `__context__` is None. `raise X from Y`: `__cause__` is Y
  (any exception instance, even a never-raised constructed one — observed printed
  traceback for `from e` where `e = ValueError("constructed only")`),
  `__suppress_context__` is True. `raise X from None` sets
  `__suppress_context__` True and `__cause__` None — the context traceback is then NOT
  printed at all.
- **S-02-35 (MUST)** **Exact uncaught-exception stderr format.** For exception E with
  context/cause chains, CPython prints chain elements outermost-first (deepest cause
  first), recursive. Grammar (pinned, byte-exact requirement):
  - One chain link = zero-or-more frame lines + final summary line:
    - `Traceback (most recent call last):` (first line of each link);
    - per frame, two lines:
      `  File "<path>", line <N>, in <name>` then the source line with exactly
      4-space indent, then (3.11+ FGE) a marker line (S-02-37);
      `<name>` is the function's `__qualname__`-derived frame name (`<module>` at
      top level, `m` for a method — qualname suffix, `<lambda>` for lambdas);
    - summary line `<ExcType.__name__>: <str(exc)>`, or bare `<ExcType.__name__>`
      when str(exc) is empty (observed `AssertionError` with no message, `ValueError()`).
  - Between links, EXACTLY one preamble line, each followed by a blank line:
    - implicit context: `\nDuring handling of the above exception, another exception
      occurred:\n\n` (blank line before AND after the preamble text line);
    - explicit cause: `\nThe above exception was the direct cause of the following
      exception:\n\n`;
  - a cause/context exception with NO traceback prints only its summary line (no
    `Traceback` header) before the preamble (observed A02-9: `TypeError: cause` then
    blank, then preamble, then `Traceback...` for the ValueError);
  - multi-link chains repeat (observed triple chain: two "During handling" preambles,
    each link fully printed oldest→newest);
  - the whole report goes to STDERR; stdout written before the exception remains
    (observed STDOUT-MARK with traceback purely on stderr); trailing newline after
    the summary line; exit code 1.
  Observed canonical single-link report (file `/p/x.py`, line 3 in `<module>`):
  ```
  Traceback (most recent call last):
    File "/tmp/speccheck/anchors/A02-12.py", line 3, in <module>
      f(1, "x")
      ~^^^^^^^^
    File "/tmp/speccheck/anchors/A02-12.py", line 2, in f
      return a + b
             ~~^~~
  TypeError: unsupported operand type(s) for +: 'int' and 'str'
  ```
- **S-02-36 (MUST)** Frame lines are emitted OUTERMOST-first (module level first,
  the raising callee last) — matching the header "most recent call last". Generator
  frames appear when a yield raised (observed: the `list(g())` caller frame with
  `~~~~^^^^` marker followed by the generator's frame).
- **S-02-37 (MUST)** Fine-grained error location marker line (3.11+ behavior,
  required in v1). On the line after the indented source line, a marker line made of
  `~` (continuation/inactive), `^` (active anchor) and spaces, aligned to the
  viewport CPython computes:
  - Binary/arithmetic errors anchor the OPERATOR: `return a + b` → `~~~^~~`
    (leading tildes under left operand, `^` at the `+` column, trailing carets under
    right operand); `1 / 0` → `~~^~~`; single-token-width operands `1/0` → `~^~`;
  - Call errors anchor the callee and args: `f(1)` missing-arg → `~^^^`;
    `f(1, "x")` → `~^^^^^^^^`; `int("nope")` → `~~~^^^^^^^^`; nested call
    `print(str(boom()))` marks the inner call `~~~~^^`; zero-arg call `f()` → `~^^`;
  - Name access: `priint("hi")` → `^^^^^^` (under the name only, no `~`);
    attribute miss `c.nope` → `^^^^^^`; subscript `xs[99]` → `~~^^^^`;
  - Multi-line calls emit one fragment line per source line with the 3.14 stacking
    (observed 5-line marker sequence for `f(\n table=..., \n rows="z",\n)`);
  - `raise ...` statements and simple statements: NO marker line (observed:
    `raise ValueError("x")` frame has source line but no caret line);
  - assert: `assert False and print("never")` → marker `^^^^^^^^^^^^^^^^^^^^^^^^`
    (span of the whole test expression).
  For v1: markers MUST match CPython on every corpus case; any divergence in the
  marker line (columns, `~`/`^`, presence) is a conformance bug (master.md gate rule).
  Behavior beyond corpus (viewport heuristics for >256-column views,
  truncated lines) is UNSPECIFIED but SHOULD degrade the same way CPython does.
- **S-02-38 (MUST)** Frame name rules: module-level is `<module>`; functions use the
  qualified name's innermost part (methods print `in m`, not `in C.m`; nested funcs
  print `in inner`, observed) — CPython uses `co_name`, not `__qualname__`. Lambdas
  are `<lambda>`.

## 7. assert

- **S-02-39 (MUST)** `assert TEST` raises `AssertionError()` when bool(TEST) is
  False, with empty str; `assert TEST, MSG` raises `AssertionError` with exactly one
  arg MSG (so `str(e)` is `str(MSG)` — observed `AssertionError: custom message`);
  MSG is evaluated ONLY when the assertion fails (observed: msg function not called
  when assert passes). `assert` runs under the default interpreter flags (v1 always
  runs with `__debug__` True; `-O` elision is out of corpus scope).
  AssertionError is an ordinary Exception subclass — catchable by `except
  AssertionError`, traceback format per S-02-35/37.

## 8. with / context managers

- **S-02-40 (MUST)** `with EXPR as TARGET:` evaluates EXPR, calls
  `type(EXPR).__enter__` (NOT instance attr; S-03 lookup rules apply), binds TARGET
  to the return value, runs the body, and ALWAYS calls `__exit__(exc_type,
  exc_value, exc_tb)` on the object (attribute lookup on the instance for exit is
  also via the type per CPython; v1 corpus relies on normal methods only).
  Multiple items `with A() as a, B() as b:` nest: B entered after A; on any error,
  exits run in reverse entry order.
- **S-02-41 (MUST)** If the body raises and `__exit__` returns a truthy value, the
  exception is SUPPRESSED (not propagated; chaining state is cleared) — observed:
  swallow returns True → execution continues after the with. Falsy/None return
  propagates the original exception unchanged (identity preserved).
  `__exit__` receives `(None, None, None)` on the clean path (observed `exit: None`
  printed by an exit handler).
- **S-02-42 (MUST)** Exceptions raised inside `__exit__` REPLACE a pending body
  exception and chain it as `__context__` (implicit chaining). An object without
  `__enter__` raises `TypeError: 'int' object does not support the context manager protocol (missed
  __exit__ method)` when neither dunder exists (CPython names `__exit__` first in
  the both-missing case); when only `__enter__` is missing:
  `TypeError: 'NoEnter' object does not support the context manager protocol (missed
  __enter__ method)` (observed both forms).
  `with EXPR:` without `as` ignores the enter value.

---

## Corpus anchors

Scripts run as files via `python3.14 <abs-path>`; the `File "..."` line echoes the
invocation path — test cases in the corpus will have their own absolute path, and
`ap-conform` compares after normalizing the two invocations to the SAME path text.
All quoted stdout/stderr bytes below are verbatim CPython 3.14.7 output. Anchors
below show outputs recorded with the anchor file path shown; keep path identical
between the two implementations.

1. `A02-01-legb-shadowing.py` (S-02-1) —
   `x = "g"` / `def f():` / `    x = "l"` / `    def i():` / `        return x` /
   `    return i()` / `print(f(), x)` →
   stdout `l g⏎`, exit 0.
2. `A02-02-def-defaults-and-params.py` (S-02-7/8) —
   `def mk(seq=[]):` / `    seq.append(1)` / `    return seq` /
   `def f(a, b=2, *args, c, d=4, **kw):` / `    return (a, b, args, c, d, sorted(kw))` /
   `print(mk(), mk())` / `print(f(1, 3, 5, c=7, e=8))` →
   stdout `[1, 1] [1, 1]⏎(1, 3, (5,), 7, 4, ['e'])⏎`, exit 0.
3. `A02-03-missing-kwonly.py` (S-02-10, 35, 36, 37) —
   `def f(a, *, c):` / `    return c` / `f(1)` →
   stdout `` (empty), exit 1, stderr:
   ```
   Traceback (most recent call last):
     File "/tmp/speccheck/anchors/A02-03.py", line 3, in <module>
       f(1)
       ~^^^
   TypeError: f() missing 1 required keyword-only argument: 'c'
   ```
   (path text = invocation path)
4. `A02-04-loop-else.py` (S-02-16/18) —
   `for i in range(3):` / `    if i == 1:` / `        break` / `else:` /
   `    print("no")` / `n = 0` / `while n < 2:` / `    n += 1` / `else:` /
   `    print("while-else", n)` →
   stdout `while-else 2⏎`, exit 0.
5. `A02-05-yield-send-return.py` (S-02-21/22) —
   `def g():` / `    x = yield 1` / `    return x + 1` / `gg = g()` /
   `print(next(gg))` / `try:` / `    gg.send(10)` / `except StopIteration as e:` /
   `    print("stop", e.value)` →
   stdout `1⏎stop 11⏎`, exit 0.
6. `A02-06-pep479.py` (S-02-23) —
   `def g():` / `    yield next(iter([]))` / `try:` / `    list(g())` /
   `except RuntimeError as e:` /
   `    print(type(e).__name__, ":", e, "| cause:", type(e.__cause__).__name__)` →
   stdout `RuntimeError : generator raised StopIteration | cause: StopIteration⏎`,
   exit 0.
7. `A02-07-implicit-chaining.py` (S-02-34/35/37) —
   `try:` / `    1 / 0` / `except ZeroDivisionError:` / `    int("nope")` →
   stdout ``, exit 1, stderr (byte-exact):
   ```
   Traceback (most recent call last):
     File "/tmp/speccheck/anchors/A02-07.py", line 2, in <module>
       1 / 0
       ~~^~~
   ZeroDivisionError: division by zero

   During handling of the above exception, another exception occurred:

   Traceback (most recent call last):
     File "/tmp/speccheck/anchors/A02-07.py", line 4, in <module>
       int("nope")
       ~~~^^^^^^^^
   ValueError: invalid literal for int() with base 10: 'nope'
   ```
8. `A02-08-explicit-chaining.py` (S-02-34/35) —
   `raise ValueError("bad") from TypeError("cause")` →
   stdout ``, exit 1, stderr (byte-exact — note the cause has no Traceback header
   because it was never raised):
   ```
   TypeError: cause

   The above exception was the direct cause of the following exception:

   Traceback (most recent call last):
     File "/tmp/speccheck/anchors/A02-08.py", line 1, in <module>
       raise ValueError("bad") from TypeError("cause")
   ValueError: bad
   ```
9. `A02-09-return-in-finally.py` (S-02-33) —
    `def f():` / `    try:` / `        return 1` / `    finally:` / `        return 2` /
    `print(f())` →
    stdout `2⏎`, exit 0, stderr:
    ```
    /tmp/speccheck/anchors/A02-09.py:5: SyntaxWarning: 'return' in a 'finally' block
      return 2
    ```
10. `A02-10-assert-message.py` (S-02-39/29) —
    `try:` / `    assert False, "custom message"` / `except AssertionError as e:` /
    `    print("AssertionError:", e)` →
    stdout `AssertionError: custom message⏎`, exit 0.
11. `A02-11-with-suppress.py` (S-02-40/41) —
    `class C:` / `    def __enter__(self):` / `        return "v"` /
    `    def __exit__(self, t, v, tb):` / `        print("exit:", t.__name__)` /
    `        return True` / `with C() as v:` / `    print("body", v)` /
    `    raise ValueError("x")` / `print("after")` →
    stdout `body v⏎exit: ValueError⏎after⏎`, exit 0.
12. `A02-12-binary-op-tb-markers.py` (S-02-35/36/37/38) —
    `def f(a, b):` / `    return a + b` / `f(1, "x")` →
    stdout ``, exit 1, stderr (byte-exact):
    ```
    Traceback (most recent call last):
      File "/tmp/speccheck/anchors/A02-12.py", line 3, in <module>
        f(1, "x")
        ~^^^^^^^^
      File "/tmp/speccheck/anchors/A02-12.py", line 2, in f
        return a + b
               ~~^~~
    TypeError: unsupported operand type(s) for +: 'int' and 'str'
    ```
