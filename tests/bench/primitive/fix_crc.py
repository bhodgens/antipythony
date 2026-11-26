#!/usr/bin/env python3
"""fix_crc.py -- byte-exactness differential gate for crc32/adler32
(leaf pp2-crc, plans/primitive-perf 02-parity-audit fix wave).

Usage:
    python3.14 fix_crc.py --other <cmd...>   e.g.
    python3.14 fix_crc.py --other /path/to/antipythony run
    (no --other = self mode; the OTHER leg runs this same file as a
    subprocess in WORKER mode via FIX_CRC_WORKER/FIX_CRC_OUT env).

Input: the driver pre-generates DATAFILE (deterministic 64-bit LCG,
identical for both sides; default path = probe dir / ppcrc_data.bin,
or argv[1] / FIX_CRC_DATAFILE) so the antipythony leg does no
interpreter-visible blob generation (the per-byte gen floor is ~25us/
byte there -- a 1MiB in-interpreter gen took >170s; CPython 40ms).
Sizes 0/1/64/1KiB/48KiB/1MiB are slices of the one 1MiB file.

Covers (all deterministic):
- zlib.crc32 / binascii.crc32 / zlib.adler32 on every size
- value chaining: crc32(b, crc32(a)) == crc32(a+b) and the adler32
  analogue, plus 3-chunk chains and explicit-0-seed first chunks
- seed semantics: crc32(d) == crc32(d, 0); adler32(d) == adler32(d, 1);
  negative and >2^32 seed values (mask-and-wrap like CPython's u32 seed)
- binascii.crc32 == zlib.crc32 identity on every size and chained calls
- unsigned 32-bit result type (CPython returns plain int, 0..2^32-1)
- error texts for bad arg types (str/list data / str / float / None value)
- edge: b'' with and without explicit seeds

Exit 0 = green, 1 = mismatch(es). Prints one JSON summary line.
"""

import argparse
import json
import os
import subprocess
import sys
import tempfile
import zlib
import binascii

HERE = os.path.dirname(os.path.abspath(__file__))


def datafile_path():
    if len(sys.argv) > 1 and not sys.argv[1].startswith("-"):
        return sys.argv[1]
    env = os.environ.get("FIX_CRC_DATAFILE")
    if env:
        return env
    return os.path.join(HERE, "ppcrc_data.bin")


def lcg_file(path, total):
    """Deterministic 1MiB LCG byte file (documented generator; regenerate
    with: python3.14 fix_crc.py --gen <path>)."""
    out = bytearray()
    x = 0x243F6A885A97D5C3
    mask = 0xFFFFFFFFFFFFFFFF
    while len(out) < total:
        x = (x * 6364136223846793005 + 1442695040888963407) & mask
        out.extend(x.to_bytes(8, "big"))
    with open(path, "wb") as f:
        f.write(out[:total])


def load_blobs():
    path = datafile_path()
    if not os.path.exists(path):
        if os.environ.get("FIX_CRC_WORKER") == "1":
            raise SystemExit("WORKER-MISSING-DATAFILE " + path)
        lcg_file(path, 1024 * 1024)
    with open(path, "rb") as f:
        blob = f.read()
    sizes = [0, 1, 64, 1024, 48 * 1024, 1024 * 1024]
    return {n: blob[:n] for n in sizes}


def run_case(fn):
    try:
        return ("ok", repr(fn()))
    except Exception as e:  # noqa: BLE001 - deliberate broad catch
        return ("err", type(e).__name__ + ": " + str(e))


def build_cases(blobs):
    """Yield (case_id, kind, payload) triples; dispatch executes them.

    NO closures over loop variables (antipythony default-arg capture
    bug): payloads are plain data, kinds are plain strings.
    """
    cases = []
    sizes = [0, 1, 64, 1024, 48 * 1024, 1024 * 1024]

    for n in sizes:
        d = blobs[n]
        cases.append(("crc.%d" % n, "crc32", d))
        cases.append(("crc0.%d" % n, "crc32seed", (d, 0)))
        cases.append(("abin.crc.%d" % n, "bcrc32", d))
        cases.append(("adler.%d" % n, "adler32", d))
        cases.append(("adler1.%d" % n, "adler32seed", (d, 1)))
        cases.append(("adler0.%d" % n, "adler32seed", (d, 0)))

    # chaining: crc32(b, crc32(a)) == crc32(a+b), 3-chunk chains
    a = blobs[1024]
    b = blobs[48 * 1024]
    c = blobs[64]
    full = a + b + c
    cases.append(("chain.crc3", "crc32chain", (a, b, c)))
    cases.append(("chain.crc3full", "crc32", full))
    cases.append(("chain.adler3", "adler32chain", (a, b, c)))
    cases.append(("chain.adler3full", "adler32", full))
    # chain with explicit 0 seed on the first chunk (CPython: identical)
    cases.append(("chain.crc0first", "crc32chain0", (a, b, c)))
    cases.append(("chain.adler0first", "adler32chain0", (a, b, c)))

    # seed wrap semantics: negative and >2^32 seeds
    d = blobs[64]
    cases.append(("seed.crc.neg", "crc32seed", (d, -1)))
    cases.append(("seed.crc.big", "crc32seed", (d, 2 ** 33 + 5)))
    cases.append(("seed.adler.neg", "adler32seed", (d, -1)))
    cases.append(("seed.adler.big", "adler32seed", (d, 2 ** 33 + 5)))

    # binascii identity on chained calls
    cases.append(("id.binchain", "bcrc32chain", (a, b, c)))

    # result range/unsignedness
    cases.append(("type.crc", "typecrc", d))
    cases.append(("type.adler", "typeadler", d))

    # error texts (bad arg types)
    cases.append(("err.crc.str", "err", ("crc32str",)))
    cases.append(("err.crc.list", "err", ("crc32list",)))
    cases.append(("err.crc.valstr", "err", ("crc32valstr",)))
    cases.append(("err.crc.valflt", "err", ("crc32valflt",)))
    cases.append(("err.crc.valnone", "err", ("crc32valnone",)))
    cases.append(("err.adler.str", "err", ("adler32str",)))
    cases.append(("err.adler.valstr", "err", ("adler32valstr",)))
    cases.append(("err.bin.str", "err", ("bcrc32str",)))
    cases.append(("err.bin.valnone", "err", ("bcrc32valnone",)))

    return cases


def dispatch(kind, payload):
    if kind == "crc32":
        return zlib.crc32(payload)
    if kind == "crc32seed":
        d, v = payload
        return zlib.crc32(d, v)
    if kind == "bcrc32":
        return binascii.crc32(payload)
    if kind == "adler32":
        return zlib.adler32(payload)
    if kind == "adler32seed":
        d, v = payload
        return zlib.adler32(d, v)
    if kind == "crc32chain":
        a, b, c = payload
        return zlib.crc32(c, zlib.crc32(b, zlib.crc32(a)))
    if kind == "crc32chain0":
        a, b, c = payload
        return zlib.crc32(c, zlib.crc32(b, zlib.crc32(a, 0)))
    if kind == "bcrc32chain":
        a, b, c = payload
        return binascii.crc32(c, binascii.crc32(b, binascii.crc32(a)))
    if kind == "adler32chain":
        a, b, c = payload
        return zlib.adler32(c, zlib.adler32(b, zlib.adler32(a)))
    if kind == "adler32chain0":
        a, b, c = payload
        return zlib.adler32(c, zlib.adler32(b, zlib.adler32(a, 0)))
    if kind == "typecrc":
        v = zlib.crc32(payload)
        return (type(v).__name__, 0 <= v < 2 ** 32)
    if kind == "typeadler":
        v = zlib.adler32(payload)
        return (type(v).__name__, 0 <= v < 2 ** 32)
    if kind == "err":
        tag = payload[0]
        try:
            if tag == "crc32str":
                zlib.crc32("string")  # type: ignore[arg-type]
            elif tag == "crc32list":
                zlib.crc32([1, 2, 3])  # type: ignore[arg-type]
            elif tag == "crc32valstr":
                zlib.crc32(b"x", "s")  # type: ignore[arg-type]
            elif tag == "crc32valflt":
                zlib.crc32(b"x", 1.5)  # type: ignore[arg-type]
            elif tag == "crc32valnone":
                zlib.crc32(b"x", None)  # type: ignore[arg-type]
            elif tag == "adler32str":
                zlib.adler32("string")  # type: ignore[arg-type]
            elif tag == "adler32valstr":
                zlib.adler32(b"x", "s")  # type: ignore[arg-type]
            elif tag == "bcrc32str":
                binascii.crc32("string")  # type: ignore[arg-type]
            elif tag == "bcrc32valnone":
                binascii.crc32(b"x", None)  # type: ignore[arg-type]
            return "NO-ERROR"
        except (TypeError, ValueError) as e:
            return type(e).__name__ + ": " + str(e)
    raise AssertionError("unknown kind " + kind)


def collect():
    blobs = load_blobs()
    out = []
    for cid, kind, payload in build_cases(blobs):
        r = run_case(lambda k=kind, p=payload: dispatch(k, p))
        out.append([cid, r[0], r[1]])
    return out


def worker_main():
    with open(os.environ["FIX_CRC_OUT"], "w") as f:
        json.dump(collect(), f)
    return 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("datafile", nargs="?", default=None,
                    help="path of the pre-generated data file")
    ap.add_argument("--gen", metavar="PATH",
                    help="generate the 1MiB LCG data file at PATH and exit")
    ap.add_argument("--other", nargs="+", default=None,
                    help="interpreter command prefix for the OTHER leg, "
                         "e.g. /path/to/antipythony run")
    args = ap.parse_args()

    if args.gen:
        lcg_file(args.gen, 1024 * 1024)
        return 0

    if args.other is None:
        other = collect()
        other_label = "self"
    else:
        with tempfile.NamedTemporaryFile(suffix=".json", delete=False) as tf:
            out_path = tf.name
        env = dict(os.environ)
        env["FIX_CRC_WORKER"] = "1"
        env["FIX_CRC_OUT"] = out_path
        env["FIX_CRC_DATAFILE"] = datafile_path()
        cmd = args.other + [os.path.abspath(__file__)]
        try:
            proc = subprocess.run(cmd, env=env, capture_output=True,
                                  text=True, timeout=600)
        except subprocess.TimeoutExpired:
            print(json.dumps({"probe": "diff_crc", "cases": 0,
                              "mismatches": ["OTHER-TIMEOUT after 600s"],
                              "rc": 1}))
            return 1
        if proc.returncode != 0:
            print(json.dumps({"probe": "diff_crc", "cases": 0,
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
        "probe": "diff_crc",
        "cases": len(mine),
        "mismatches": mismatches,
        "other": other_label,
    }
    print(json.dumps(summary))
    return 1 if mismatches else 0


if __name__ == "__main__":
    if os.environ.get("FIX_CRC_WORKER") == "1":
        sys.exit(worker_main())
    sys.exit(main())
