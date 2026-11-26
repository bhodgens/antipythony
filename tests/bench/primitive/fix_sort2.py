#!/usr/bin/env python3
"""fix_sort2.py -- differential gate for leaf pp2-sort (audit Fix B:
per-compare special_lookup hoist + raw homogeneous comparator arms).

Usage:
    python3.14 fix_sort2.py --other <cmd...>     e.g.
    python3.14 fix_sort2.py --other /path/to/antipythony
    (no --other = self mode; the OTHER leg runs this same file as a
    subprocess in FIX_SORT2_WORKER/FIX_SORT2_OUT env mode, mirroring
    fix_sort.py's harness shape).

What this gate pins (correctness of the perf rework; values AND error
type+text byte-identical vs CPython 3.14):
1. Homogeneous int/str/float sorts at 10/10k/100k sizes through the new
   raw comparator arms must stay CPython-identical (order + stability).
2. Mixed-type lists still raise the EXACT CPython TypeError on both
   sorted() and list.sort() (raw-arm escalation must not swallow it).
3. key=str / key=len / key=None / reverse=True semantics unchanged.
4. Stability: equal keys keep input order, including under reverse=True
   (decorate-reverse-sort-reverse scheme preserved).
5. NaN / -0.0 / inf float ordering (CPython 3.10+ total order: NaNs
   last ascending, first under reverse; -0.0 == 0.0 ties keep input
   order and repr distinguishes them).
6. Bigints (beyond smi range) sort exactly on raw arms; bool/int/float
   numeric mixes sort CPython-identically.
7. Surrogate-bearing strs sort in code-point order (V016 semantics).

Seeded data via a deterministic LCG -- NO random.Random dependency.

Exit 0 = green, 1 = mismatch(es). Prints one JSON summary line.
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
        out.append(x)
    return out


def seeded_strs(n, salt):
    ints = seeded_ints(n, salt)
    return ["k" + str(i) for i in ints]


def build_matrix():
    """(case_id, kind, payload) triples; kinds are plain strings."""
    cases = []

    # --- A: homogeneous families at 10 / 10k / 100k (raw arms + hoist) ---
    for n, salt, tag in ((10, 1, "10"), (10_000, 2, "10k"), (100_000, 3, "100k")):
        ints = seeded_ints(n, salt)
        strs = seeded_strs(n, salt + 40)
        floats = [i - n // 2 for i in ints]
        cases.append((f"A.int{tag}.sorted", "sorted", list(ints)))
        cases.append((f"A.int{tag}.lsort", "lsort", list(ints)))
        cases.append((f"A.int{tag}.sorted_rev", "sorted_rev", list(ints)))
        cases.append((f"A.int{tag}.lsort_rev", "lsort_rev", list(ints)))
        cases.append((f"A.str{tag}.sorted", "sorted", list(strs)))
        cases.append((f"A.str{tag}.lsort", "lsort", list(strs)))
        cases.append((f"A.str{tag}.sorted_rev", "sorted_rev", list(strs)))
        cases.append((f"A.float{tag}.sorted", "sorted", list(floats)))
        cases.append((f"A.float{tag}.lsort", "lsort", list(floats)))
        cases.append((f"A.float{tag}.sorted_rev", "sorted_rev", list(floats)))

    # --- B: exact mixed-type TypeErrors (raw arms must escalate, not swallow) ---
    cases.append(("B.mixed_si.sorted", "sorted", [3, "a", 1]))
    cases.append(("B.mixed_si.lsort", "lsort", [3, "a", 1]))
    cases.append(("B.mixed_sn.sorted", "sorted", [1, 2, "x", None]))
    cases.append(("B.mixed_sn.lsort", "lsort", [1, 2, "x", None]))
    cases.append(("B.mixed_snone.sorted", "sorted", ["a", None, "b"]))
    cases.append(("B.mixed_snone.lsort", "lsort", ["a", None, "b"]))
    cases.append(("B.mixed_bs.sorted", "sorted", [True, "x"]))
    cases.append(("B.mixed_fs.sorted", "sorted", [1.0, "x"]))
    cases.append(("B.mixed_fs.lsort", "lsort", [1.0, "x"]))
    cases.append(("B.mixed_tail.sorted", "sorted", [5, 4, 3, 2, 1, 0, "a"]))
    cases.append(("B.mixed_tail.lsort", "lsort", [5, 4, 3, 2, 1, 0, "a"]))
    cases.append(("B.key_none_mixed.sorted", "sorted_key_none", [3, "a", 1]))

    # --- C: key= / key=None / reverse= semantics ---
    ints10k = seeded_ints(10_000, 7)
    strs10k = seeded_strs(10_000, 8)
    cases.append(("C.key_str.sorted", "sorted_key_str", list(ints10k)))
    cases.append(("C.key_str.lsort", "lsort_key_str", list(ints10k)))
    cases.append(("C.key_len.sorted", "sorted_key_len", list(strs10k)))
    cases.append(("C.key_len.lsort", "lsort_key_len", list(strs10k)))
    cases.append(("C.key_none.int", "sorted_key_none", list(ints10k)))
    cases.append(("C.key_none.str", "sorted_key_none", list(strs10k)))
    cases.append(("C.key_none.lsort_int", "lsort_key_none", list(ints10k)))
    cases.append(("C.rev_false.sorted", "sorted_rev_false", list(ints10k)))
    cases.append(("C.rev_false.lsort", "lsort_rev_false", list(ints10k)))
    cases.append(("C.key_len_mixed.sorted", "sorted_key_len_mixed",
                  ["aaa", "bb", "c", 12, 7]))

    # --- D: stability (equal keys keep input order; reverse=True too) ---
    tuples = [("b", 1), ("a", 2), ("b", 0), ("a", 1), ("b", 5), ("a", 9)]
    cases.append(("D.stable.key", "sorted_key_stable", list(tuples)))
    cases.append(("D.stable.lsort_key", "lsort_key_stable", list(tuples)))
    cases.append(("D.stable.rev", "sorted_key_rev_stable", list(tuples)))
    cases.append(("D.stable.lsort_rev", "lsort_key_rev_stable", list(tuples)))
    # equal-ELEMENT stability on the raw arms (identity-repr-distinguishable
    # only via key wrappers; plain dup ints are value-identical)
    cases.append(("D.stable.dupkey_rev", "sorted_key_rev_stable",
                  [("x", 1), ("x", 2), ("x", 3)]))

    # --- E: NaN / -0.0 / inf float total order (CPython 3.10+) ---
    finf = float("inf")
    fninf = float("-inf")
    fnan = float("nan")
    cases.append(("E.nan.sorted", "sorted", [1.0, fnan, 0.5, fnan, -2.0]))
    cases.append(("E.nan.lsort", "lsort", [1.0, fnan, 0.5, fnan, -2.0]))
    cases.append(("E.nan.sorted_rev", "sorted_rev", [1.0, fnan, 0.5, fnan, -2.0]))
    cases.append(("E.nan.lsort_rev", "lsort_rev", [1.0, fnan, 0.5, fnan, -2.0]))
    cases.append(("E.nan.mixed_num", "sorted", [1, 2.5, fnan, True]))
    cases.append(("E.nan.mixed_num_lsort", "lsort", [1, 2.5, fnan, True]))
    cases.append(("E.negzero.sorted", "sorted", [0.0, -0.0, 0.0, -0.0]))
    cases.append(("E.negzero.sorted_rev", "sorted_rev", [0.0, -0.0, 0.0, -0.0]))
    cases.append(("E.negzero.lsort_rev", "lsort_rev", [0.0, -0.0, 0.0, -0.0]))
    cases.append(("E.inf.sorted", "sorted", [finf, -fninf, 0.0, fninf, finf]))
    cases.append(("E.inf.sorted_rev", "sorted_rev", [finf, -fninf, 0.0, fninf, finf]))

    # --- F: bigints (beyond smi range) + numeric mixes ---
    big1 = 10 ** 20
    big2 = 2 ** 59
    big3 = -10 ** 20
    cases.append(("F.big.sorted", "sorted", [big1, 5, big2, big3, 0]))
    cases.append(("F.big.lsort", "lsort", [big1, 5, big2, big3, 0]))
    cases.append(("F.big.sorted_rev", "sorted_rev", [big1, 5, big2, big3, 0]))
    cases.append(("F.bigfloat.sorted", "sorted", [big1, 0.5, big3]))
    cases.append(("F.bigfloat.lsort", "lsort", [big1, 0.5, big3]))
    cases.append(("F.boolint.sorted", "sorted", [True, False, True, False, True]))
    cases.append(("F.boolint.sorted_rev", "sorted_rev", [True, False, True]))
    cases.append(("F.boolfloat.sorted", "sorted", [True, 0.5, False, -1.5]))
    cases.append(("F.boolfloat.lsort", "lsort", [True, 0.5, False, -1.5]))
    cases.append(("F.nummix.sorted", "sorted", [1, 2.5, 3, True, 0]))
    cases.append(("F.nummix.lsort", "lsort", [1, 2.5, 3, True, 0]))

    # --- G: surrogate strs (code-point order) ---
    cases.append(("G.surr.sorted", "sorted", ["\ud800z", "a", "\ud800a", "\ud800b"]))
    cases.append(("G.surr.lsort", "lsort", ["\ud800z", "a", "\ud800a", "\ud800b"]))
    cases.append(("G.surr.sorted_rev", "sorted_rev", ["\ud800z", "a", "\ud800a"]))

    # --- H: trivial shapes through every arm ---
    cases.append(("H.empty.sorted", "sorted", []))
    cases.append(("H.empty.lsort", "lsort", []))
    cases.append(("H.single.sorted", "sorted", [42]))
    cases.append(("H.single.str.sorted", "sorted", ["x"]))
    cases.append(("H.single.float.sorted", "sorted", [1.5]))
    cases.append(("H.dups.sorted", "sorted", [3, 1, 2, 3, 1, 2, 3]))
    cases.append(("H.already.sorted", "sorted", list(range(50))))
    cases.append(("H.revrange.sorted", "sorted", list(range(50, 0, -1))))

    return cases


def dispatch(kind, payload):
    if kind == "sorted":
        return sorted(payload)
    if kind == "lsort":
        l = list(payload)
        l.sort()
        return l
    if kind == "sorted_rev":
        return sorted(payload, reverse=True)
    if kind == "lsort_rev":
        l = list(payload)
        l.sort(reverse=True)
        return l
    if kind == "sorted_key_none":
        return sorted(payload, key=None)
    if kind == "lsort_key_none":
        l = list(payload)
        l.sort(key=None)
        return l
    if kind == "sorted_key_len":
        return sorted(payload, key=len)
    if kind == "lsort_key_len":
        l = list(payload)
        l.sort(key=len)
        return l
    if kind == "sorted_key_str":
        return sorted(payload, key=str)
    if kind == "lsort_key_str":
        l = list(payload)
        l.sort(key=str)
        return l
    if kind == "sorted_rev_false":
        return sorted(payload, reverse=False)
    if kind == "lsort_rev_false":
        l = list(payload)
        l.sort(reverse=False)
        return l
    if kind == "sorted_key_len_mixed":
        return sorted(payload, key=len)
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
    raise AssertionError("unknown kind " + kind)


def collect():
    out = []
    for cid, kind, payload in build_matrix():
        r = run_case(lambda: dispatch(kind, payload))
        out.append([cid, r[0], r[1]])
    return out


def worker_main():
    with open(os.environ["FIX_SORT2_OUT"], "w") as f:
        json.dump(collect(), f)
    return 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--other", nargs="+", default=None,
                    help="interpreter command prefix for the OTHER leg")
    args = ap.parse_args()

    if args.other is None:
        other = collect()
        other_label = "self"
    else:
        with tempfile.NamedTemporaryFile(suffix=".json", delete=False) as tf:
            out_path = tf.name
        env = dict(os.environ)
        env["FIX_SORT2_WORKER"] = "1"
        env["FIX_SORT2_OUT"] = out_path
        cmd = args.other + [os.path.abspath(__file__)]
        try:
            proc = subprocess.run(cmd, env=env, capture_output=True,
                                  text=True, timeout=600)
        except subprocess.TimeoutExpired:
            print(json.dumps({"probe": "fix_sort2", "cases": 0,
                              "mismatches": ["OTHER-TIMEOUT after 600s"],
                              "rc": 1}))
            return 1
        if proc.returncode != 0:
            print(json.dumps({"probe": "fix_sort2", "cases": 0,
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
            mismatches.append({"case": m[0], "ref": f"{m[1]} {m[2][:200]}",
                               "other": f"{o[1]} {o[2][:200]}"})
    if len(mine) != len(other):
        mismatches.append({"case": "<length>", "ref": str(len(mine)),
                           "other": str(len(other))})

    summary = {
        "probe": "fix_sort2",
        "cases": len(mine),
        "mismatches": mismatches,
        "other": other_label,
    }
    print(json.dumps(summary))
    return 1 if mismatches else 0


if __name__ == "__main__":
    if os.environ.get("FIX_SORT2_WORKER") == "1":
        sys.exit(worker_main())
    sys.exit(main())
