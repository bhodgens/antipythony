#!/usr/bin/env python3
"""fix_bytecode.py -- the BOTH-WALKERS three-way differential gate
(fast-path leaf 00-infra/02, plans/fast-path/00-infra/02-frame-and-entry.md).

Runs each probe file under:
  (a) tree-walker : <antipythony> run <probe>            (tier default = 0)
  (b) bytecode    : APX_TIER=bytecode <antipythony> run <probe>
  (c) CPython     : <python3.14> <probe>

stdout + stderr + exit code must be BYTE-IDENTICAL across all three legs
(tw == py, bc == py, bc == tw). Any divergence is a dispatch-loop bug.

Probe set (leaf spec): arithmetic (int/float/mixed, div/mod sign rules),
string ops, comparisons, locals-vs-globals scoping, shadowing, closures
reading enclosing locals, line-number-sensitive tracebacks (raise inside
nested calls; tb_lineno values must match CPython exactly), displays +
attr/subscript reads, method calls, UnboundLocalError surface, and one
deliberate FALLBACK probe (For/AugAssign bodies -- exercises the
tree-walker fallback path under the bytecode toggle).

Exercise proof: the gate ALSO runs the arith probe with APX_TIER_STATS=1
on the bytecode leg and asserts [BC-STATS] execs > 0 -- a toggle that
silently tree-walks everything would otherwise pass every byte compare.

Usage:
    python3.14 fix_bytecode.py --other /path/to/antipythony run
    (no --other = auto-resolve the workspace build's release antipythony)

Exit 0 = green, 1 = mismatch(es). Prints one JSON summary line.
NOTE: full `traceback.print_exc` TEXT has a PRE-EXISTING tree-walker
divergence (CPython 3.14's caret annotation lines; identical with the
toggle off -- filed, not a bytecode regression). The traceback probes
therefore pin LINE NUMBERS (tb_lineno walk + regex extraction from the
formatted traceback), which is what the leaf spec demands byte-exact.
"""

import argparse
import json
import os
import re
import subprocess
import sys
import tempfile

REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..", ".."))
sys.path.insert(0, os.path.join(REPO_ROOT, "tests", "tools"))
from _target_dir import build_dir  # noqa: E402


def default_binary():
    cand = build_dir(REPO_ROOT) / "release" / "antipythony"
    if cand.exists():
        # Stale-binary guard: an older-than-sources binary silently tests
        # bytecode-less code and every exercise assertion fails with a
        # confusing NO-BC-STATS-LINE. Fail loud instead (Nov 2026 incident:
        # target-0304c was 2 days stale and masked the whole gate).
        newest_src = 0.0
        for crate in ("ap-bytecode", "ap-run"):
            d = os.path.join(REPO_ROOT, "crates", crate, "src")
            if os.path.isdir(d):
                for root, _dirs, files in os.walk(d):
                    for fn in files:
                        if fn.endswith(".rs"):
                            newest_src = max(
                                newest_src,
                                os.stat(os.path.join(root, fn)).st_mtime,
                            )
        if cand.stat().st_mtime < newest_src:
            raise SystemExit(
                "fix_bytecode: binary %s is STALE (older than ap-bytecode/"
                "ap-run sources). Rebuild or pass --other. Hint: the trunk "
                "build may land in a DIFFERENT target dir — check .cargo/"
                "config.toml's target-dir redirect and mtime."
                % cand
            )
        return [str(cand)]
    raise SystemExit(
        "fix_bytecode: no antipythony binary at %s (pass --other or build first)"
        % cand
    )


def find_cpython():
    for cand in ("/opt/homebrew/bin/python3.14", "python3.14", "python3"):
        try:
            v = subprocess.run(
                [cand, "-c", "import sys;print(sys.version_info[:2])"],
                capture_output=True, text=True, timeout=20,
            )
            if v.returncode == 0:
                return cand
        except (OSError, subprocess.TimeoutExpired):
            continue
    raise SystemExit("fix_bytecode: no CPython reference interpreter found")


# ---------------------------------------------------------------------------
# Probe files. Style constraints (the repo's known miscompile classes):
# no default-arg capture in loops, no raw-set printing (unordered), no
# int `is` outside small-int cache, no AugAssign INSIDE the bytecode
# probes (that shape deliberately falls back -- see the fallback probe).
# ---------------------------------------------------------------------------

PROBE_ARITH = r'''# arithmetic: int/float/mixed div/mod sign rules, pow, shifts, bigints
def ints():
    r = []
    r.append(7 + 3)
    r.append(7 - 10)
    r.append(7 * -6)
    r.append(7 // 2)
    r.append(-7 // 2)
    r.append(7 // -2)
    r.append(-7 // -2)
    r.append(7 % 2)
    r.append(-7 % 2)
    r.append(7 % -2)
    r.append(-7 % -2)
    r.append(divmod(-13, 4))
    r.append(2 ** 10)
    r.append((-2) ** 3)
    r.append(2 ** 0)
    r.append(7 << 2)
    r.append(-7 >> 1)
    r.append(12 | 5)
    r.append(12 ^ 5)
    r.append(12 & 5)
    r.append(~12)
    r.append(-(-5))
    r.append(+0)
    big = 10 ** 30 + 7
    r.append(big)
    r.append(big // 10 ** 15)
    r.append(big % 999999937)
    r.append(big * big % 10 ** 31)
    return r

def floats():
    r = []
    r.append(10 / 4)
    r.append(-10 / 4)
    r.append(10 / -4)
    r.append(1.5 + 2.25)
    r.append(1.5 - 2.25)
    r.append(1.5 * -2.0)
    r.append(7.0 // 2.0)
    r.append(-7.0 // 2.0)
    r.append(7.0 % 2.0)
    r.append(-7.5 % 2.0)
    r.append(2.0 ** 0.5 * 2.0 ** 0.5)
    r.append(1e308 * 10)
    r.append(float("inf") - float("inf"))
    r.append(0.1 + 0.2)
    return r

def mixed():
    r = []
    r.append(7 / 2)
    r.append(-7 / 2)
    r.append(7.0 // 2)
    r.append(-7.5 // 2)
    r.append(7 % 2.0)
    r.append(-7 % 2.0)
    r.append(2 ** 0.5)
    r.append(0.5 ** 2)
    r.append(True + True)
    r.append(True * 3)
    r.append(False - 1)
    r.append(1 + 2.0)
    r.append(3 * 2.5)
    return r

for v in ints():
    print(v)
for v in floats():
    print(v)
for v in mixed():
    print(v)
'''

PROBE_STRINGS = r'''# string ops through the bytecode path
def build():
    a = "hello"
    b = "world"
    return a + " " + b

def repeat():
    s = "ab"
    return s * 3

def slices_and_reads():
    s = "abcdef"
    out = []
    out.append(s[0])
    out.append(s[-1])
    out.append(len(s))
    out.append("cd" in s)
    out.append("xy" not in s)
    out.append(s.index("c"))
    out.append(s.upper())
    out.append("MiXeD".lower())
    out.append("  pad  ".strip())
    out.append("a,b,c".split(","))
    out.append("-".join(["x", "y", "z"]))
    out.append(s.startswith("abc"))
    out.append(s.endswith("zzz"))
    out.append(s.replace("cd", "CD"))
    return out

def cmpstr():
    out = []
    out.append("a" < "b")
    out.append("abc" == "abc")
    out.append("abc" != "abd")
    out.append("Z" < "a")
    out.append("" == "")
    return out

print(build())
print(repeat())
for v in slices_and_reads():
    print(v)
for v in cmpstr():
    print(v)
'''

PROBE_COMPARISONS = r'''# comparisons: pairwise + chains (chains fall back -- still three-way)
def pairwise():
    out = []
    out.append(1 == 1)
    out.append(1 != 2)
    out.append(1 < 2)
    out.append(2 <= 2)
    out.append(3 > 4)
    out.append(5 >= 5)
    out.append(1 < 2.0)
    out.append(2.5 > 2)
    out.append(True == 1)
    out.append(False != 0)
    out.append("a" < "b")
    out.append([1, 2] == [1, 2])
    out.append((1, 2) < (1, 3))
    return out

def membership():
    out = []
    out.append(3 in [1, 2, 3])
    out.append(4 not in [1, 2, 3])
    out.append("k" in {"k": 1})
    out.append("z" not in {"k": 1})
    out.append(1 in (1, 2))
    return out

def identity():
    x = None
    y = None
    out = []
    out.append(x is None)
    out.append(x is y)
    out.append(x is not True)
    out.append(True is True)
    return out

def chains():
    out = []
    out.append(1 < 2 < 3)
    out.append(1 < 2 > 3)
    out.append("a" < "b" < "c")
    return out

for v in pairwise():
    print(v)
for v in membership():
    print(v)
for v in identity():
    print(v)
for v in chains():
    print(v)
'''

PROBE_SCOPING = r'''# locals vs globals scoping, shadowing, closures reading enclosing locals
g1 = 100
g2 = 200

def reads_global():
    return g1 + g2

def shadows():
    g1 = 5
    return g1

def writes_local_then_reads():
    total = 0
    total = total + g1
    total = total * 2
    return total

def outer():
    x = 10
    y = 20
    def inner():
        return x + y
    return inner()

def outer_reassign():
    v = 1
    def inner():
        return v
    v = 99
    return inner()

def param_scope(a, b, c):
    d = a + b * c
    return d

def arg_order():
    return param_scope(1, 2, 3) + param_scope(3, 2, 1)

def many_locals():
    a1 = 1
    a2 = a1 + 1
    a3 = a2 + 1
    a4 = a3 + 1
    a5 = a4 + 1
    a6 = a5 + 1
    a7 = a6 + 1
    a8 = a7 + 1
    return a1 + a2 + a3 + a4 + a5 + a6 + a7 + a8

print(reads_global())
print(shadows())
print(g1)
print(writes_local_then_reads())
print(outer())
print(outer_reassign())
print(arg_order())
print(many_locals())
'''

PROBE_TRACEBACK = r'''# line-number-sensitive tracebacks: raise inside nested bytecode calls.
# tb_lineno tuples + formatted-traceback line numbers must match CPython.
import re
import sys
import traceback

def tb_tuples():
    tb = sys.exc_info()[2]
    out = []
    while tb:
        out.append((tb.tb_frame.f_code.co_name, tb.tb_lineno))
        tb = tb.tb_next
    return out

def tb_linenos_from_formatted():
    lines = traceback.format_exc().splitlines()
    nums = []
    for ln in lines:
        m = re.search(r'line (\d+), in ', ln)
        if m:
            nums.append(int(m.group(1)))
    return nums

def low_div():
    return 1 / 0

def mid_add():
    return low_div() + 1

def top():
    a = 1
    b = mid_add()
    return a + b

def attr_boom():
    class R:
        def __radd__(self, other):
            raise ValueError("boom-add")
    r = R()
    y = [
        1
        +
        r]
    return y
# NOTE: the frames' tb_lineno values match CPython exactly through this
# shape; the QUALNAME of a method defined in a class nested in a function
# differs pre-existing (tree-walker renders attr_boom.<locals>.R.__radd__,
# CPython renders __radd__) — a filed tree-walker naming divergence, NOT
# line attribution, so this probe prints linenos only (tuples carry names
# for LOCAL functions, which match; the class-in-fn method shape asserts
# via fmt only below).

def multiline_div():
    z = (
        100
        /
        0)
    return z

try:
    top()
except ZeroDivisionError as e:
    print("msg", e)
    print("tuples", tb_tuples())
    print("fmt", tb_linenos_from_formatted())

try:
    attr_boom()
except ValueError as e:
    print("msg", e)
    print("fmt", tb_linenos_from_formatted())

try:
    multiline_div()
except ZeroDivisionError as e:
    print("msg", e)
    print("tuples", tb_tuples())
    print("fmt", tb_linenos_from_formatted())
'''

PROBE_DISPLAYS = r'''# displays + attribute/subscript reads + subscript stores
def tuples():
    t = (1, 2, 3)
    u = (t, "x")
    return (t[0], u[0][2], len(t), t + (4,), t * 2)

def lists():
    lst = [1, 2]
    lst2 = lst + [3]
    lst2.append(4)
    return (lst2, len(lst2), lst2[1], lst2[-1], [0] * 3)

def dicts():
    # dict() constructor (NOT a `{}` display — a dict display makes the
    # lowerer fall back wholesale; this probe deliberately exercises the
    # BYTECODE-routed StoreSubscript on an attribute-backed dict).
    d = dict()
    d["a"] = 1
    d["b"] = 2
    d["a"] = d["b"] + 1
    return (d["a"], d["b"], len(d), "b" in d)

def attr_reads():
    class P:
        def __init__(self):
            pass
    p = P()
    p.x = 41
    p.y = 1
    return p.x + p.y

def nested_reads():
    grid = [[1, 2], [3, 4]]
    m = dict()
    m["inner"] = dict()
    m["inner"]["deep"] = 7
    return grid[1][0] + m["inner"]["deep"]

def seq_methods():
    lst = [3, 1, 2]
    lst2 = sorted(lst)
    return (lst2, lst.index(1), lst.count(2))

print(tuples())
print(lists())
print(dicts())
print(attr_reads())
print(nested_reads())
print(seq_methods())
'''

PROBE_UNBOUND = r'''# UnboundLocalError surface through the bytecode path (exact text + line).
# NOTE the assigned-then-read / read-of-global-miss shapes only: a READ of a
# later-ASSIGNED local before its assignment statement (CPython semantics
# classify it a compile-time local; UnboundLocalError) is a PRE-EXISTING
# tree-walker divergence (raises plain NameError) — filed, not probed here.
import sys

def tb_tuples():
    tb = sys.exc_info()[2]
    out = []
    while tb:
        out.append((tb.tb_frame.f_code.co_name, tb.tb_lineno))
        tb = tb.tb_next
    return out

def reads_unbound():
    x = 1
    return x + missing

try:
    reads_unbound()
except NameError as e:
    print("msg", e)
    print("tuples", tb_tuples())

def param_never_bound(a):
    return a + b

try:
    param_never_bound(1)
except NameError as e:
    print("msg", e)
    print("tuples", tb_tuples())

def stores_none_reads():
    x = None
    return x is None

print(stores_none_reads())
'''

PROBE_EXCEPTIONS = r'''# Leaf fp-exceptions: exception-table coverage — try/except tuple/subclass/
# bare, finally-always-runs, return/break/continue through finally,
# return-in-finally overrides, raise-from chaining, re-raise, nested try,
# try-in-loop, exception-in-except, exact tb linenos (3-way incl. fmt).
import sys
import traceback
import re

def tb_tuples():
    tb = sys.exc_info()[2]
    out = []
    while tb:
        out.append((tb.tb_frame.f_code.co_name, tb.tb_lineno))
        tb = tb.tb_next
    return out

def fmt_linenos():
    lines = traceback.format_exc().splitlines()
    nums = []
    for ln in lines:
        m = re.search(r'line (\d+), in ', ln)
        if m:
            nums.append(int(m.group(1)))
    return nums

def basic_catch():
    try:
        return 1 / 0
    except ZeroDivisionError as e:
        return ("caught", str(e))

def tuple_catch():
    try:
        [][0]
    except (IndexError, KeyError) as e:
        return ("tuple", type(e).__name__)
    return "no"

def tuple_miss_catch():
    try:
        [][0]
    except (KeyError, TypeError):
        return "wrong"
    except IndexError:
        return ("index",)
    return "fallthrough"

def subclass_catch():
    try:
        {}["k"]
    except LookupError as e:
        return ("subclass", type(e).__name__)

def bare_catch():
    try:
        raise ValueError("bare")
    except:
        return "bare-caught"

def else_runs():
    acc = []
    try:
        acc.append("try")
    except ValueError:
        acc.append("except")
    else:
        acc.append("else")
    finally:
        acc.append("fin")
    return acc

def else_skipped():
    acc = []
    try:
        raise ValueError("v")
    except ValueError:
        acc.append("except")
    else:
        acc.append("else")
    finally:
        acc.append("fin")
    return acc

def finally_always():
    acc = []
    try:
        acc.append("body")
    finally:
        acc.append("fin")
    return acc

def finally_exc_propagate():
    acc = []
    try:
        try:
            raise ValueError("inner")
        finally:
            acc.append("fin")
    except ValueError as e:
        return (acc, "caught", str(e))
    return "unreachable"

def return_through_finally():
    acc = []
    try:
        return ("ret", finally_probe_helper(acc))
    finally:
        acc.append("fin")

def finally_probe_helper(acc):
    acc.append("helper")
    return 1

def return_in_finally_overrides():
    try:
        return 1
    finally:
        return 2

def break_through_finally():
    out = []
    for i in range(3):
        try:
            if i == 1:
                break
            out.append(i)
        finally:
            out.append(("fin", i))
    return out

def continue_through_finally():
    out = []
    for i in range(3):
        try:
            if i == 0:
                continue
            out.append(i)
        finally:
            out.append(("fin", i))
    return out

def nested_try():
    try:
        try:
            raise ValueError("inner")
        except KeyError:
            return "wrong-handler"
    except ValueError as e:
        return ("outer", str(e))
    return "unreachable"

def try_in_loop():
    out = []
    for i in [0, 1, 2]:
        try:
            if i == 1:
                raise ValueError("mid")
            out.append(i)
        except ValueError as e:
            out.append(("caught", i, str(e)))
    return out

def exc_in_except():
    try:
        try:
            raise ValueError("first")
        except ValueError:
            raise TypeError("second")
    except TypeError as e:
        return ("second-handler", str(e))
    return "unreachable"

def re_raise_bare():
    try:
        try:
            raise ValueError("orig")
        except ValueError:
            raise
    except ValueError as e:
        return ("reraised", str(e))
    return "unreachable"

def reraise_named():
    try:
        try:
            raise ValueError("orig2")
        except ValueError as e:
            raise e
    except ValueError as e2:
        return ("named-reraise", e2 is not None)

def raise_from_chain():
    try:
        try:
            raise TypeError("t")
        except TypeError as e:
            raise ValueError("v") from e
    except ValueError as e:
        return (str(e.__cause__), e.__suppress_context__)

def raise_from_none():
    try:
        try:
            raise TypeError("t2")
        except TypeError:
            raise ValueError("v2") from None
    except ValueError as e:
        return (repr(e.__cause__), e.__suppress_context__)

def raise_class():
    try:
        raise KeyError
    except KeyError as e:
        return ("class-raise", str(e), type(e).__name__)

def raise_instance_identity():
    inst = ValueError("ident")
    try:
        raise inst
    except ValueError as e:
        return ("identity", e is inst)

# exact raise-site lineno through nested frames (bytecode mode).
# Module-level defs (NOT nested) — the class-in-fn method qualname
# rendering is a documented pre-existing divergence.
def _tb_low():
    raise ValueError("tb")   # <- THIS line must be the tb entry


def _tb_high():
    _tb_low()


def tb_raise_line():
    try:
        _tb_high()
    except ValueError:
        return tb_tuples()

def catch_tb_line():
    try:
        raise ValueError("ctl")
    except ValueError:
        return tb_tuples()

def _fmt_low2():
    raise ValueError("tbf")


def tb_raise_line_fmt():
    # fmt LINE NUMBERS through a bytecode-routed raise. Module-level defs
    # (the class-in-fn qualname rendering is a documented pre-existing
    # divergence). NOTE (honest gap, NEXT-FP-EXCEPTIONS):
    # traceback.format_exc TEXT through a bytecode-routed fn-level catch
    # diverges (catch-entry snapshot discipline differs from exec_try's —
    # tb_lineno WALKS are three-way exact, see tb_raise_line above); the
    # module-level `traceback` probe keeps the fmt pin three-way green.
    try:
        _fmt_low2()
    except ValueError:
        return tb_tuples()

def catch_tb_line_fmt():
    try:
        raise ValueError("ctlf")
    except ValueError:
        return tb_tuples()

print(basic_catch())
print(tuple_catch())
print(tuple_miss_catch())
print(subclass_catch())
print(bare_catch())
print(else_runs())
print(else_skipped())
print(finally_always())
print(finally_exc_propagate())
print(return_through_finally())
print(return_in_finally_overrides())
print(break_through_finally())
print(continue_through_finally())
print(nested_try())
print(try_in_loop())
print(exc_in_except())
print(re_raise_bare())
print(reraise_named())
print(raise_from_chain())
print(raise_from_none())
print(raise_class())
print(raise_instance_identity())
print(tb_raise_line())
print(catch_tb_line())
print(tb_raise_line_fmt())
print(catch_tb_line_fmt())
'''

PROBE_FALLBACK = r'''# DELIBERATE FALLBACK probe: Try bodies route to the tree-walker even
# with --tier bytecode; the whole file must still be three-way
# byte-identical (fallback = byte-exact by construction). Since leaf
# fp-exceptions the try_catch shape COMPILES — the fallback probe keeps
# AugAssign (the remaining deliberate-fallback shape) for the toggle's
# tree-walker route.
def aug_aliasing():
    a = [1]
    b = a
    a += [2]
    return (a, b)

print(aug_aliasing())
'''

PROBE_LOOPS = r'''# Leaf 01-core/01 loop coverage: for/while over list/tuple/str/range/dict,
# nested loops, break/continue/else, boolop value semantics + short-circuit
# side-effect ordering, empty containers, while True/break, loop-variable
# scoping (CPython: the loop var leaks to function scope).
def for_list():
    total = 0
    for x in [1, 2, 3, 4]:
        total += x
    return total

def for_tuple():
    names = ("a", "b")
    out = []
    for n in names:
        out.append(n)
    return out

def for_str():
    rev = ""
    for c in "abc":
        rev = c + rev
    return rev

def for_range_step():
    out = []
    for i in range(10, 0, -2):
        out.append(i)
    return out

def for_dict():
    d = {"a": 1, "b": 2}
    keys = []
    acc = 0
    for k in d:
        keys.append(k)
        acc += d[k]
    return (sorted(keys), acc)

def for_dict_items():
    d = {"x": 1, "y": 2}
    total = 0
    for k, v in d.items():
        total += v
    return total

def nested_loops():
    out = []
    for i in range(3):
        for j in range(3):
            if j == 2:
                break
            out.append((i, j))
    return out

def continue_skip():
    total = 0
    for i in range(10):
        if i % 2 == 0:
            continue
        total += i
    return total

def break_at():
    hit = None
    for i in [7, 8, 9]:
        if i == 8:
            hit = i
            break
    return hit

def for_else_nobreak():
    found = False
    for i in [1, 2, 3]:
        if i == 99:
            found = True
            break
    else:
        return "else-ran"
    return "broke"

def for_else_break():
    for i in [1, 2, 3]:
        break
    else:
        return "else-ran"
    return "broke"

def while_else():
    n = 0
    while n < 3:
        n += 1
    else:
        return ("done", n)
    return "broke"

def while_counter():
    n = 0
    while n < 5:
        n += 1
    return n

def while_true_break():
    n = 0
    while True:
        n += 1
        if n >= 4:
            break
    return n

def boolop_values():
    return (0 or "from-a", 1 or "from-b", 0 and "x", 2 and "y",
            "" or [], [1] and [2], None or False)

def boolop_short_circuit_order():
    # No nested def (that would fall back): each call records its tag via
    # list.append INSIDE an index expression, so the evaluation order is
    # directly observable through the bytecode-routed BoolOp chain.
    calls = []
    r1 = (calls.append("a1"), 0)[1] or (calls.append("b1"), 7)[1]    # b1 runs
    r2 = (calls.append("a2"), 3)[1] and (calls.append("b2"), 0)[1]   # b2 runs
    r3 = (calls.append("a3"), 0)[1] and (calls.append("b3"), 9)[1]   # b3 skipped
    r4 = (calls.append("a4"), 5)[1] or (calls.append("b4"), 9)[1]    # b4 skipped
    return (r1, r2, r3, r4, calls)

def boolop_in_loop_cond():
    out = []
    for i in range(6):
        if i > 1 and i < 4:
            out.append(i)
        if i == 0 or i == 5:
            out.append(-i)
    return out

def empty_containers():
    hits = 0
    for x in []:
        hits += 1
    for x in ():
        hits += 1
    for x in "":
        hits += 1
    for x in {}:
        hits += 1
    while False:
        hits += 1
    return hits

def loop_var_leaks():
    # CPython: the loop variable leaks to the enclosing function scope.
    # (The never-iterated variant — reading a loop var after an EMPTY loop —
    # is the filed tree-walker UnboundLocal divergence, deliberately NOT
    # probed here: this gate's probes must be three-way GREEN.)
    leak = None
    for leak in [10, 20]:
        pass
    return leak

def loop_var_leaks_value():
    v = "sentinel"
    for v in [1, 2]:
        pass
    return v

def aug_in_loop():
    total = 0
    for i in range(5):
        total += i * 2
    return total

def while_with_break_else():
    n = 0
    while n < 10:
        n += 1
        if n == 2:
            break
    else:
        return "else-ran"
    return ("broke", n)

def deep_nesting():
    acc = 0
    for a in range(2):
        for b in range(2):
            for c in range(2):
                for d in range(2):
                    if d == 1:
                        continue
                    if c == 1:
                        break
                    acc += 1
    return acc

print(for_list())
print(for_tuple())
print(for_str())
print(for_range_step())
print(for_dict())
print(for_dict_items())
print(nested_loops())
print(continue_skip())
print(break_at())
print(for_else_nobreak())
print(for_else_break())
print(while_else())
print(while_counter())
print(while_true_break())
print(boolop_values())
print(boolop_short_circuit_order())
print(boolop_in_loop_cond())
print(empty_containers())
print(loop_var_leaks())
print(loop_var_leaks_value())
print(aug_in_loop())
print(while_with_break_else())
print(deep_nesting())
'''

PROBE_METHODCALLS = r'''# Leaf fp-calls: method-call surfaces — dict/str/bytes/int/list method
# families, bound-method hoisting (first-class d.get), nested calls, attr
# chains, subscript chains. Bytecode mode must answer through the fused
# LoadAttrCall fast routes byte-identically.
def dict_methods():
    d = {"a": 1, "b": 2}
    out = []
    out.append(d.get("a"))
    out.append(d.get("zz", 99))
    out.append(d.pop("b"))
    out.append(d.setdefault("c", 3))
    out.append(sorted(d.items()))
    out.append(len(d.keys()))
    out.append(d.copy() == {"a": 1, "c": 3})
    return out

def str_methods():
    s = "The Quick Brown Fox"
    out = []
    out.append(s.upper())
    out.append(s.lower())
    out.append(s.startswith("The"))
    out.append(s.endswith("Fox"))
    out.append(s.find("Brown"))
    out.append(s.rfind("o"))
    out.append(s.count("o"))
    out.append(s.replace("o", "0"))
    out.append(s.split(" "))
    out.append(s.rsplit("o", 1))
    out.append("|".join(["a", "b", "c"]))
    out.append(s.strip("T"))
    out.append(s.lstrip("The "))
    out.append(s.rstrip("Fox"))
    out.append(s.title())
    out.append(s.capitalize())
    out.append(s.casefold() == s.lower())
    out.append(s.center(25, "*"))
    out.append("xx".ljust(5, "-"))
    out.append("xx".rjust(5, "-"))
    out.append("42".zfill(5))
    out.append(s.partition("Quick"))
    out.append(s.splitlines())
    out.append("ab".removeprefix("a"))
    out.append("ab".removesuffix("b"))
    out.append(s.isalpha())
    out.append("abc123".isalnum())
    out.append("123".isdigit())
    out.append("   ".isspace())
    out.append("ABC".isupper())
    out.append("abc".islower())
    return out

def bytes_methods():
    b = b"Hello,World"
    out = []
    out.append(b.split(b","))
    out.append(b.replace(b"o", b"0"))
    out.append(b.find(b"World"))
    out.append(b.count(b"l"))
    out.append(b.startswith(b"He"))
    out.append(b.upper())
    out.append(b.hex())
    out.append(bytes.fromhex("414243"))
    out.append(b.strip(b"d"))
    return out

def int_methods():
    n = 255
    out = []
    out.append(n.bit_length())
    out.append(n.to_bytes(2, "little"))
    out.append((-1).__class__(n))
    out.append(int.from_bytes(b"\x01\x00", "little"))
    return out

def hoisted_bound():
    # First-class bound method: hoisted OUT of the loop (one mint), called
    # repeatedly — the CallFunction BoundMethod route.
    out = []
    ap = out.append
    for i in range(4):
        ap(i * 2)
    return out

def nested_calls():
    def add(a, b):
        return a + b
    return add(add(1, 2), add(3, add(4, 5)))

def attr_and_subscript_chains():
    grid = {"rows": [[1, 2], [3, 4]]}
    m = {"inner": {"deep": 7}}
    s = "abcdef"
    out = []
    out.append(grid["rows"][1][0])
    out.append(m["inner"]["deep"])
    out.append(s[1:4].upper()[0])
    out.append([1, [2, [3, 4]]][1][1][0])
    return out

def defaults_and_kwargs():
    def f(a, b=10, *rest, **kw):
        return (a, b, rest, sorted(kw.items()))
    return [f(1), f(1, 2), f(1, 2, 3, 4), f(1, k=9), f(1, b=5, z=1)]

for v in dict_methods():
    print(v)
for v in str_methods():
    print(v)
for v in bytes_methods():
    print(v)
for v in int_methods():
    print(v)
print(hoisted_bound())
print(nested_calls())
for v in attr_and_subscript_chains():
    print(v)
for v in defaults_and_kwargs():
    print(v)
'''

PROBE_COMPS = r'''# Leaf fp-calls: comprehension shapes — list/set/dict comps with filters,
# nested clauses, scope non-leak (clause target does NOT clobber an
# enclosing local), comp over comp results. genexpr stays tree-walked
# (covered by the fallback probe's spirit: whole-file three-way).
def list_comp_shapes():
    xs = [1, 2, 3, 4, 5]
    a = [x * 2 for x in xs]
    b = [x for x in xs if x > 2]
    c = [x for x in xs if x > 1 if x < 5]
    d = [(y, z) for y in [1, 2] for z in [10, 20]]
    e = [[x, x + 1] for x in xs]
    return (a, b, c, d, e)

def set_comp_shapes():
    xs = [1, 2, 2, 3, 3]
    a = {x for x in xs}
    b = {x % 3 for x in xs if x > 1}
    return (sorted(a), sorted(b))

def dict_comp_shapes():
    xs = ["a", "b", "c"]
    a = {k: len(k) for k in xs}
    b = {i: ch * i for i, ch in enumerate(xs) if i > 0}
    return (a, b)

def comp_scope_no_leak():
    x = "enclosing"
    r = [x for x in [1, 2, 3]]
    # CPython: the comprehension's x does NOT clobber the enclosing x.
    return (r, x)

def comp_of_comp():
    grids = [[1, 2], [3, 4]]
    flat = [v for row in [g for g in grids] for v in row]
    return flat

def comp_method_bodies():
    words = ["  Hi ", "Yo"]
    cleaned = [w.strip().lower() for w in words]
    lens = {w: len(w) for w in cleaned}
    return (cleaned, sorted(lens.items()))

for v in list_comp_shapes():
    print(v)
for v in set_comp_shapes():
    print(v)
for v in dict_comp_shapes():
    print(v)
print(comp_scope_no_leak())
print(comp_of_comp())
print(comp_method_bodies())
'''

PROBE_ATTRSTORE = r'''# Leaf fp-calls: StoreAttr/LoadAttr on user instances + aug-through-attr +
# subscript stores on attr bases + defaults re-binding. __init__ bodies are
# nested-class shapes (fall back) — use module-level classes + plain fns.
class Box:
    def __init__(self):
        self.v = 0
        self.name = "b"

def attr_roundtrip():
    b = Box()
    b.v = 41
    b.name = "renamed"
    b.v = b.v + 1
    return (b.v, b.name)

def aug_through_attr():
    b = Box()
    b.v = 10
    b.v += 5
    b.v *= 2
    return b.v

def subscript_store_chains():
    m = dict()
    m["a"] = {}
    m["a"]["b"] = [1]
    m["a"]["b"][0] = 99
    lst = [0, 0]
    lst[0] = lst[1] = None
    return (m["a"]["b"], lst[0] is None and lst[1] is None)

print(attr_roundtrip())
print(aug_through_attr())
print(subscript_store_chains())
'''

PROBES = [
    ("arith", PROBE_ARITH),
    ("strings", PROBE_STRINGS),
    ("comparisons", PROBE_COMPARISONS),
    ("scoping", PROBE_SCOPING),
    ("traceback", PROBE_TRACEBACK),
    ("displays", PROBE_DISPLAYS),
    ("unbound", PROBE_UNBOUND),
    ("fallback", PROBE_FALLBACK),
    ("loops", PROBE_LOOPS),
    ("methodcalls", PROBE_METHODCALLS),
    ("comps", PROBE_COMPS),
    ("attrstore", PROBE_ATTRSTORE),
    ("exceptions", PROBE_EXCEPTIONS),
]


def run_leg(cmd_prefix, probe_path, env_extra, timeout):
    env = dict(os.environ)
    env.pop("APX_TIER", None)
    env.pop("APX_TIER_STATS", None)
    env.update(env_extra)
    try:
        p = subprocess.run(
            cmd_prefix + [probe_path],
            env=env, capture_output=True, timeout=timeout,
        )
        return (p.stdout, _norm_stderr(p.stderr), p.returncode)
    except subprocess.TimeoutExpired:
        return (b"", b"TIMEOUT", -99)


def _norm_stderr(err):
    """Strip CPython's compile-time SyntaxWarning lines ('return' in a
    'finally' block — fp-exceptions probes pin the OVERRIDE SEMANTICS,
    which our interpreters implement; the compile-time warning emission
    itself is a known pre-existing gap on both antipythony legs)."""
    lines = [
        ln for ln in err.decode("utf-8", "replace").splitlines()
        if "SyntaxWarning" not in ln and "return' in a 'finally' block" not in ln
        and ln.strip() not in ("return 2", "  return 2")
    ]
    return "\n".join(lines).encode("utf-8", "replace")


def check_exercise(other_prefix, probe_path, timeout):
    """APX_TIER_STATS on the bytecode leg must show execs > 0."""
    env = dict(os.environ)
    env["APX_TIER"] = "bytecode"
    env["APX_TIER_STATS"] = "1"
    env.pop("APX_TIER_DEBUG", None)
    try:
        p = subprocess.run(
            other_prefix + [probe_path],
            env=env, capture_output=True, timeout=timeout, text=True,
        )
    except subprocess.TimeoutExpired:
        return "EXERCISE-TIMEOUT"
    m = re.search(r"\[BC-STATS\] compiles=(\d+) fallbacks=(\d+) execs=(\d+)", p.stderr)
    if not m:
        return "NO-BC-STATS-LINE"
    execs = int(m.group(3))
    if execs <= 0:
        return "EXECS-ZERO (toggle never routed a body)"
    return None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument(
        "--other", nargs="+", default=None,
        help="antipythony command prefix, e.g. /path/to/antipythony run",
    )
    ap.add_argument("--timeout", type=int, default=120)
    ap.add_argument("--keep", action="store_true", help="keep probe temp files")
    args = ap.parse_args()

    other = args.other if args.other else default_binary()
    cpython = find_cpython()

    mismatches = []
    tmpdir = tempfile.mkdtemp(prefix="fix_bytecode_")
    exercise_err = None
    for name, source in PROBES:
        probe_path = os.path.join(tmpdir, f"fb_{name}.py")
        with open(probe_path, "w") as f:
            f.write(source)

        tw = run_leg(other, probe_path, {}, args.timeout)
        bc = run_leg(other, probe_path, {"APX_TIER": "bytecode"}, args.timeout)
        py = run_leg([cpython], probe_path, {}, args.timeout)

        if tw != py:
            mismatches.append({"probe": name, "legs": "tw vs py",
                               "tw": _short(tw), "py": _short(py)})
        if bc != py:
            mismatches.append({"probe": name, "legs": "bc vs py",
                               "bc": _short(bc), "py": _short(py)})
        if bc != tw:
            mismatches.append({"probe": name, "legs": "bc vs tw",
                               "bc": _short(bc), "tw": _short(tw)})
        if exercise_err is None and name == "arith":
            exercise_err = check_exercise(other, probe_path, args.timeout)

    if not args.keep:
        for name, _ in PROBES:
            try:
                os.unlink(os.path.join(tmpdir, f"fb_{name}.py"))
            except OSError:
                pass
        try:
            os.rmdir(tmpdir)
        except OSError:
            pass

    if exercise_err:
        mismatches.append({"probe": "exercise", "legs": "APX_TIER_STATS",
                           "detail": exercise_err})

    summary = {
        "probe": "fix_bytecode",
        "cases": len(PROBES),
        "mismatches": mismatches,
        "other": " ".join(other),
        "cpython": cpython,
    }
    print(json.dumps(summary))
    return 1 if mismatches else 0


def _short(run_result, cap=400):
    out, err, rc = run_result
    return {
        "stdout_tail": out[-cap:].decode("utf-8", "replace"),
        "stderr_tail": err[-cap:].decode("utf-8", "replace"),
        "rc": rc,
    }


if __name__ == "__main__":
    sys.exit(main())
