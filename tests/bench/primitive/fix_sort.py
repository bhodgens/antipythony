#!/usr/bin/env python3
"""fix_sort.py -- differential gate for leaf ppfix-sort
(plans/primitive-perf/02-parity-audit fix leaves: the 3 sort
correctness bugs).

Usage:
    python3.14 fix_sort.py --other <cmd...>     e.g.
    python3.14 fix_sort.py --other /path/to/antipythony
    (no --other = self mode; the OTHER leg runs this same file as a
    subprocess in FIX_SORT_WORKER/FIX_SORT_OUT env mode, mirroring
    diff_b64.py's harness shape).

Bugs this gate catches (RED until fixed, BY DESIGN):
1. sorted() swallows mixed-type compare TypeErrors
   (sorted([3,'a',1]) returned input order; CPython raises
   TypeError: '<' not supported between instances of 'str' and 'int').
2. list.sort() v0 homogeneous arm silently no-ops mixed lists.
3. sorted(.., key=None) / list.sort(key=None) crash
   ('NoneType' object is not callable); CPython treats key=None as
   no-key.

Seeded lists built WITHOUT random.Random (antipythony's native random
module lacks the class - separate leaf): deterministic arithmetic
patterns, reversed ranges, pip patterns.

Exit 0 = green, 1 = mismatch(es). Prints one JSON summary line.
Order AND exception type+text are compared byte-identically against
CPython 3.14.
"""

import argparse
import json
import os
import subprocess
import sys
import tempfile


def run_case(fn):
    try:
        return ("ok", repr(fn()))
    except Exception as e:  # noqa: BLE001 - deliberate broad catch
        return ("err", type(e).__name__ + ": " + str(e))


def seeded_ints(n, salt):
    # deterministic LCG; no random module dependency
    x = (n * 2654435761 + salt * 97 + 12345) & 0x7FFFFFFF
    out = []
    for _ in range(n):
        x = (x * 1103515245 + 12345) & 0x7FFFFFFF
        out.append(x % 1000)
    return out


def seeded_strs(n, salt):
    ints = seeded_ints(n, salt)
    return ["k" + str(i) for i in ints]


def build_matrix():
    """Yield (case_id, kind, payload) triples; dispatcher executes them.

    NO closures over loop variables (antipythony default-arg capture
    bug): payloads are plain data, kinds are plain strings.
    """
    cases = []
    ints100 = seeded_ints(100, 1)
    strs100 = seeded_strs(100, 2)
    floats50 = [i / 7 for i in range(50)] + [i / 3 for i in range(50)]

    # --- A: sorted() basic families
    cases.append(("A.sorted.ints", "sorted", list(ints100)))
    cases.append(("A.sorted.strs", "sorted", list(strs100)))
    cases.append(("A.sorted.floats", "sorted", list(floats50)))
    cases.append(("A.sorted.empty", "sorted", []))
    cases.append(("A.sorted.single", "sorted", [42]))
    cases.append(("A.sorted.dups", "sorted", [3, 1, 2, 3, 1, 2, 3]))
    cases.append(("A.sorted.already", "sorted", list(range(50))))
    cases.append(("A.sorted.revrange", "sorted", list(range(50, 0, -1))))
    cases.append(("A.sorted.tuples", "sorted",
                  [(1, "b"), (1, "a"), (0, "z"), (0, "y")]))

    # --- B: THE BUGS - mixed-type compare (must raise TypeError)
    cases.append(("B.sorted.mixed_si", "sorted", [3, "a", 1]))
    cases.append(("B.sorted.mixed_sn", "sorted", [1, 2, "x", None]))
    cases.append(("B.sorted.mixed_if", "sorted", [1, 2.5, 3]))  # int/float OK
    cases.append(("B.sorted.mixed_bif", "sorted", [True, 0, 2.5]))
    cases.append(("B.sorted.mixed_snone", "sorted", ["a", None, "b"]))
    cases.append(("B.lsort.mixed_si", "lsort", [3, "a", 1]))
    cases.append(("B.lsort.mixed_sn", "lsort", [1, 2, "x", None]))
    cases.append(("B.lsort.mixed_snone", "lsort", ["a", None, "b"]))

    # --- C: THE BUG - key=None must mean no-key
    cases.append(("C.sorted.key_none", "sorted_key_none", list(ints100)))
    cases.append(("C.lsort.key_none", "lsort_key_none", [3, 1, 2]))

    # --- D: key= / reverse= correctness
    cases.append(("D.sorted.key_len", "sorted_key_len", list(strs100)))
    cases.append(("D.sorted.key_str", "sorted_key_str", list(ints100)))
    cases.append(("D.sorted.key_neg", "sorted_key_neg", list(ints100)))
    cases.append(("D.sorted.rev_true", "sorted_rev", list(ints100)))
    cases.append(("D.sorted.rev_false", "sorted_rev_false", list(ints100)))
    cases.append(("D.lsort.key_len", "lsort_key_len", list(strs100)))
    cases.append(("D.lsort.key_str", "lsort_key_str", list(ints100)))
    cases.append(("D.lsort.rev_true", "lsort_rev", list(ints100)))
    cases.append(("D.lsort.rev_false", "lsort_rev_false", list(ints100)))
    cases.append(("D.sorted.key_mixed", "sorted_key_len_mixed",
                  ["aaa", "bb", "c", 12, 7]))  # key=len: int/str compare!
    cases.append(("D.lsort.key_mixed", "lsort_key_len_mixed",
                  ["aaa", "bb", "c", 12, 7]))
    cases.append(("D.sorted.key_none_mixed", "sorted_key_none_mixed",
                  [3, "a", 1]))  # key=None then mixed compare raises

    # --- E: stability (equal keys preserve input order)
    tuples = [("b", 1), ("a", 2), ("b", 0), ("a", 1), ("b", 5), ("a", 9)]
    cases.append(("E.stable.key", "sorted_key_stable", list(tuples)))
    cases.append(("E.stable.lsort_key", "lsort_key_stable", list(tuples)))
    cases.append(("E.stable.rev", "sorted_key_rev_stable", list(tuples)))
    cases.append(("E.stable.lsort_rev", "lsort_key_rev_stable", list(tuples)))

    # --- F: user __lt__ dispatch (must keep working - no regression)
    cases.append(("F.userlt.sorted", "sorted_userlt", None))
    cases.append(("F.userlt.lsort", "lsort_userlt", None))
    cases.append(("F.userlt.error", "sorted_userlt_error", None))

    # --- G: cmp_to_key interop (02.04 surface must not regress)
    cases.append(("G.cmptokey.sorted", "sorted_cmptokey", list(ints100)))
    cases.append(("G.cmptokey.lsort", "lsort_cmptokey", list(ints100)))

    # --- H: bad key callable / bad kwarg error surfaces
    cases.append(("H.kw.bad", "sorted_bad_kw", list(ints100)))
    cases.append(("H.kw.lsort_bad", "lsort_bad_kw", [3, 1, 2]))

    return cases


def dispatch(kind, payload):
    if kind == "sorted":
        return sorted(payload)
    if kind == "lsort":
        l = list(payload)
        l.sort()
        return l
    if kind == "sorted_key_none":
        return sorted(payload, key=None)
    if kind == "lsort_key_none":
        l = list(payload)
        l.sort(key=None)
        return l
    if kind == "sorted_key_none_mixed":
        return sorted(payload, key=None)
    if kind == "sorted_key_len":
        return sorted(payload, key=len)
    if kind == "sorted_key_str":
        return sorted(payload, key=str)
    if kind == "sorted_key_neg":
        return sorted(payload, key=lambda x: -x)
    if kind == "sorted_rev":
        return sorted(payload, reverse=True)
    if kind == "sorted_rev_false":
        return sorted(payload, reverse=False)
    if kind == "lsort_key_len":
        l = list(payload)
        l.sort(key=len)
        return l
    if kind == "lsort_key_str":
        l = list(payload)
        l.sort(key=str)
        return l
    if kind == "lsort_rev":
        l = list(payload)
        l.sort(reverse=True)
        return l
    if kind == "lsort_rev_false":
        l = list(payload)
        l.sort(reverse=False)
        return l
    if kind == "sorted_key_len_mixed":
        return sorted(payload, key=len)
    if kind == "lsort_key_len_mixed":
        l = list(payload)
        l.sort(key=len)
        return l
    if kind == "sorted_key_stable":
        return sorted(payload, key=lambda t: t[0])
    if kind == "lsort_key_stable":
        l = list(payload)
        l.sort(key=lambda t: t[0])
        return l
    if kind == "sorted_key_rev_stable":
        return sorted(payload, key=lambda t: t[0], reverse=True)
    if kind == "lsort_key_rev_stable":
        l = list(payload)
        l.sort(key=lambda t: t[0], reverse=True)
        return l
    if kind == "sorted_userlt":
        class P:
            def __init__(self, v):
                self.v = v
            def __lt__(self, other):
                return self.v < other.v
            def __repr__(self):
                return "P(" + str(self.v) + ")"
        return sorted([P(3), P(1), P(2)])
    if kind == "lsort_userlt":
        class P:
            def __init__(self, v):
                self.v = v
            def __lt__(self, other):
                return self.v < other.v
            def __repr__(self):
                return "P(" + str(self.v) + ")"
        l = [P(3), P(1), P(2)]
        l.sort()
        return l
    if kind == "sorted_userlt_error":
        class Q:
            def __init__(self, v):
                self.v = v
            def __repr__(self):
                return "Q(" + str(self.v) + ")"
        return sorted([Q(1), 2])
    if kind == "sorted_cmptokey":
        import functools
        def cmp(a, b):
            return (a > b) - (a < b)
        return sorted(payload, key=functools.cmp_to_key(cmp))
    if kind == "lsort_cmptokey":
        import functools
        def cmp(a, b):
            return (a > b) - (a < b)
        l = list(payload)
        l.sort(key=functools.cmp_to_key(cmp))
        return l
    if kind == "sorted_bad_kw":
        return sorted(payload, bogus=1)
    if kind == "lsort_bad_kw":
        l = list(payload)
        l.sort(bogus=1)
        return l
    raise AssertionError("unknown kind " + kind)


def collect():
    out = []
    for cid, kind, payload in build_matrix():
        r = run_case(lambda: dispatch(kind, payload))
        out.append([cid, r[0], r[1]])
    return out


def worker_main():
    with open(os.environ["FIX_SORT_OUT"], "w") as f:
        json.dump(collect(), f)
    return 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--other", nargs="+", default=None,
                    help="interpreter command prefix for the OTHER leg, "
                         "e.g. /path/to/antipythony")
    args = ap.parse_args()

    if args.other is None:
        other = collect()
        other_label = "self"
    else:
        with tempfile.NamedTemporaryFile(suffix=".json", delete=False) as tf:
            out_path = tf.name
        env = dict(os.environ)
        env["FIX_SORT_WORKER"] = "1"
        env["FIX_SORT_OUT"] = out_path
        cmd = args.other + [os.path.abspath(__file__)]
        try:
            proc = subprocess.run(cmd, env=env, capture_output=True,
                                  text=True, timeout=600)
        except subprocess.TimeoutExpired:
            print(json.dumps({"probe": "fix_sort", "cases": 0,
                              "mismatches": ["OTHER-TIMEOUT after 600s"],
                              "rc": 1}))
            return 1
        if proc.returncode != 0:
            print(json.dumps({"probe": "fix_sort", "cases": 0,
                              "mismatches": [f"OTHER-FAILED rc={proc.returncode}",
                                             proc.stderr[-2000:]],
                              "rc": 1}))
            return 1
        with open(out_path) as f:
            other = json.load(f)
        os.unlink(out_path)
        other_label = " ".join(args.other)

    mine = collect()

    mismatches = []
    for m, o in zip(mine, other):
        if m != o:
            mismatches.append({"case": m[0], "ref": f"{m[1]} {m[2]}",
                               "other": f"{o[1]} {o[2]}"})
    if len(mine) != len(other):
        mismatches.append({"case": "<length>", "ref": str(len(mine)),
                           "other": str(len(other))})

    summary = {
        "probe": "fix_sort",
        "cases": len(mine),
        "mismatches": mismatches,
        "other": other_label,
    }
    print(json.dumps(summary))
    return 1 if mismatches else 0


if __name__ == "__main__":
    if os.environ.get("FIX_SORT_WORKER") == "1":
        sys.exit(worker_main())
    sys.exit(main())
