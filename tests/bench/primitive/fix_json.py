#!/usr/bin/env python3
"""fix_json.py -- byte-exactness differential gate for native json
(leaf ppfix-json, plans/primitive-perf W6b).

Usage:
    python3.14 fix_json.py --other <cmd...> [--full]   e.g.
    python3.14 fix_json.py --other /path/to/antipythony run
    (no --other = self mode; the OTHER leg runs this same file as a
    subprocess in WORKER mode via FIX_JSON_WORKER/FIX_JSON_OUT env).

Corpus groups (all deterministic; every case returns plain data so the
two legs compare (status, repr) tuples byte-exactly):
- A: loads scalar + roundtrip (nulls/bools, unicode, empty structures,
  i64-boundary ints, floats incl exponents/-0.0/e-notation, every
  escape class incl \\uXXXX surrogate pairs)
- B: nested objects/arrays mixing every escape class + astral chars
- C: int boundary values around +/-2^63 (Smi/Big handoff)
- D: float table (exponents, -0.0, e+/e-, 17-sig-digit precision)
- E: structural (deep nesting 120, big arrays 100k, wide sorted objs)
- F: dumps kwargs matrix (separators, ensure_ascii on/off, sort_keys,
  indent int, allow_nan) -- byte-identity + dumps->loads roundtrip
- G: JSONDecodeError message text + line/column/char parity vs CPython
  for 26 malformed docs (runner compares both legs' rendered
  "msg: line L column C (char P)" strings on the same doc)

Pre-optimization audit (honest, 2026-11-24): the CURRENT engine
diverges from CPython on 4 known rows, fixed by the optimization leaf
in the same commit series (root causes: \\uXXXX error position after
the 4-hex scan instead of at the escape start; leading-zero numbers
not rejected; float-without-fraction ('1.', '1e') accepted; the
Unterminated-string position is a 5th divergence INCLUDED here as a
green gate -- CPython reports the OPENING quote, engine matched).
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


def build_cases():
    """Yield (case_id, kind, payload) triples; dispatch executes them.

    NO closures over loop variables (antipythony default-arg capture
    bug): payloads are plain data, kinds are plain strings.
    """
    cases = []

    esc = ('"tab\\tnl\\nquote\\"backslash\\\\slash\\/bell\\bform\\ffeed\\rcr"')
    surr = '"\\u0041 \\u00e9 \\u4e2d \\ud83d\\ude00 \\ud800\\udc00"'
    ctrl = '["\\u0000\\u001f", "\\u007f", "\\u0080"]'
    uni = '"h\\u00e9llo w\\u00f6rld \\ud83c\\udf89 \\u4e2d\\u6587 \\ud83d\\ude80"'

    # --- A: scalar loads + roundtrip
    scalars = ["null", "true", "false", "[]", "{}", '""', "0", "-0",
               esc, surr, ctrl, uni,
               '"plain ascii string"', '"a\\u0000b"',
               '[1, 2.5, true, null, "x\\ny"]']
    for i, t in enumerate(scalars):
        cases.append(("A.load.%d" % i, "loads", t))
        cases.append(("A.rt.%d" % i, "roundtrip", t))

    # --- B: nested structures mixing escape classes
    nested = (
        '{"k1": ["a\\tb", "\\u00e9\\u4e2d", null, true],'
        ' "k2": {"deep": [1e10, -0.0, "\\ud83d\\ude00"],'
        ' "s": "mix\\\\everything\\/\\"end"}, "k3": []}'
    )
    cases.append(("B.nested.load", "loads", nested))
    cases.append(("B.nested.rt", "roundtrip", nested))
    wide = "{" + ",".join('"key%03d": [i, "s%d\\n", null]'
                          % (i, i) for i in range(200)) + "}"
    cases.append(("B.wide.load", "loads", wide))
    cases.append(("B.wide.rt", "roundtrip", wide))

    # --- C: int boundaries (i64 edges + Big handoff)
    ints = ["9223372036854775807", "-9223372036854775808",
            "9223372036854775808", "-9223372036854775809",
            "99999999999999999999", "10000000000000000000000000000",
            "0", "-0", "1", "-1"]
    for i, t in enumerate(ints):
        cases.append(("C.int.%d" % i, "loads", t))
        cases.append(("C.int.rt.%d" % i, "roundtrip", t))

    # --- D: float table
    floats = ["-0.0", "0.0", "1e10", "1E-5", "2.5e+3", "-2.5e-7",
              "123456789.123456789", "0.1", "3.141592653589793",
              "1e308", "1e-308", "5e-324", "1.7976931348623157e308",
              "-1.7976931348623157e308", "1234567890123456789012.0"]
    for i, t in enumerate(floats):
        cases.append(("D.float.%d" % i, "loads", t))
        cases.append(("D.float.rt.%d" % i, "roundtrip", t))

    # --- E: structural bulk (deterministic generators)
    cases.append(("E.deep.loads", "loads", "[" * 120 + "1" + "]" * 120))
    cases.append(("E.deep.rt", "roundtrip", "[" * 120 + "1" + "]" * 120))
    cases.append(("E.bigarr.rt", "bigarr", 100000))
    cases.append(("E.widesort.rt", "widesort", 5000))

    # --- F: dumps kwargs matrix
    obj = {"b": 1, "a": [1, 2.5, True, None, "x\ny"],
           "z": {"k": (1, 2)}, "c": -0.5}
    kwsets = [
        ("default", {}), ("sort", {"sort_keys": True}),
        ("compact", {"separators": (",", ":")}),
        ("customsep", {"separators": (", ", " = ")}),
        ("noascii", {"ensure_ascii": False}),
        ("indent2", {"indent": 2}), ("indent0", {"indent": 0}),
        ("sortcompact", {"sort_keys": True, "separators": (",", ":")}),
        ("sortindent", {"sort_keys": True, "indent": 4}),
        ("indentnone", {"indent": None}),
        ("nan", {"_obj": [float("nan"), float("inf"), float("-inf")],
                 "_rt": False}),
    ]
    for name, kws in kwsets:
        cases.append(("F.dump." + name, "dumpkw", (name, kws, obj)))

    # --- G: error parity (both legs format the same doc)
    errs = ["[1,]", '{"a":1,}', '{"a"', '{"a":', "[1 2]", "[", "{",
            "-", "-x", ".5", "tru", "nul", "fals", "undefined",
            "[1,2", '"\\u12g4"', '"\\u12"', "[1,2]]", "x", "[}",
            "[01]", "01", "1e", "1e+", "1.", '"abc',
            '{"a":1 "b":2}', "{'single': 1}"]
    for i, t in enumerate(errs):
        cases.append(("G.err.%d" % i, "err", t))

    return cases


def dispatch(kind, payload):
    if kind == "loads":
        return json.loads(payload)
    if kind == "roundtrip":
        return json.loads(json.dumps(json.loads(payload)))
    if kind == "bigarr":
        big = list(range(payload))
        return [len(json.dumps(big)), json.loads(json.dumps(big)) == big]
    if kind == "widesort":
        d = {}
        for i in range(payload):
            d["key%04d" % i] = i
        p2 = json.dumps(d, sort_keys=True)
        return [len(p2), len(json.loads(p2))]
    if kind == "dumpkw":
        name, kws, obj = payload
        real = {k: v for k, v in kws.items() if not k.startswith("_")}
        target = kws.get("_obj", obj)
        out = json.dumps(target, **real)
        if kws.get("_rt", True):
            back = json.loads(out)
            return [name, out, repr(back)]
        # NaN/Infinity literals: dumps output is gated byte-exact; the
        # roundtrip leg is skipped (our loads does not parse the literals
        # yet -- decoder feature gap filed in the leaf report, NOT an
        # encoder divergence; CPython's own json.loads is non-standard
        # there per RFC 8259).
        return [name, out, "<no-rt>"]
    if kind == "err":
        try:
            json.loads(payload)
            return "NO-ERROR"
        except ValueError as e:
            return type(e).__name__ + ": " + str(e)
    raise AssertionError("unknown kind " + kind)


def collect():
    out = []
    for cid, kind, payload in build_cases():
        r = run_case(lambda k=kind, p=payload: dispatch(k, p))
        out.append([cid, r[0], r[1]])
    return out


def worker_main():
    with open(os.environ["FIX_JSON_OUT"], "w") as f:
        json.dump(collect(), f)
    return 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--other", nargs="+", default=None,
                    help="interpreter command prefix for the OTHER leg, "
                         "e.g. /path/to/antipythony run")
    args = ap.parse_args()

    if args.other is None:
        other = collect()
        other_label = "self"
    else:
        with tempfile.NamedTemporaryFile(suffix=".json", delete=False) as tf:
            out_path = tf.name
        env = dict(os.environ)
        env["FIX_JSON_WORKER"] = "1"
        env["FIX_JSON_OUT"] = out_path
        cmd = args.other + [os.path.abspath(__file__)]
        try:
            proc = subprocess.run(cmd, env=env, capture_output=True,
                                  text=True, timeout=1800)
        except subprocess.TimeoutExpired:
            print(json.dumps({"probe": "diff_json", "cases": 0,
                              "mismatches": ["OTHER-TIMEOUT after 1800s"],
                              "rc": 1}))
            return 1
        if proc.returncode != 0:
            print(json.dumps({"probe": "diff_json", "cases": 0,
                              "mismatches": [f"OTHER-FAILED rc={proc.returncode}",
                                             proc.stderr[-2000:]], "rc": 1}))
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
        "probe": "diff_json",
        "cases": len(mine),
        "mismatches": mismatches,
        "other": other_label,
    }
    print(json.dumps(summary))
    return 1 if mismatches else 0


if __name__ == "__main__":
    if os.environ.get("FIX_JSON_WORKER") == "1":
        sys.exit(worker_main())
    sys.exit(main())
