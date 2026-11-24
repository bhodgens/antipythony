#!/usr/bin/env python3
"""diff_hex.py -- byte-exactness differential gate for hexlify/unhexlify
(leaf 02-hexlify-simd T1, plans/primitive-perf/01-encode-simd).

Usage:
    python3.14 diff_hex.py --other <cmd...> [--full]   e.g.
    python3.14 diff_hex.py --other /path/to/antipythony run
    (no --other = self mode; OTHER leg runs this same file as a
    subprocess in WORKER mode via DIFF_HEX_WORKER/DIFF_HEX_OUT env).

Matrix: (A) hexlify/unhexlify every length 0..512 (odd-length encode
incl., decode odd = Odd-length error); (B) random lengths to 1 MiB
(10k full / 120 smoke, seeded bulk); (C) non-hex char at EVERY position
of 16/32/64/100-char strings + SIMD-edge positions in 16/32/64/128;
(D) upper/lowercase mixes, bytes.fromhex/bytearray.fromhex with spaces,
tabs, underscores (leading/middle/trailing); (E) exception-text
endpoints + large-input sanity.

Env notes (honest, not papered over):
- binascii imported at module level (antipythony `import base64` does
  not auto-import it; side-effect-free under CPython).
- Cases avoid default-arg capture in loops (antipythony mis-compiles
  `lambda d=data:` there); (kind, payload) tuples + dispatcher instead.

Known antipythony divergences this gate flags (gate failures BY DESIGN
until fixed): hexlify(data, sep, bytes_per_sep) ignores sep/bps;
hexlify(str) wrongly accepted; unhexlify(bytearray) wrongly rejected;
bytes.fromhex underscore error position off-by-one (after vs at).

Exit 0 = green, 1 = mismatch(es). Prints one JSON summary line.
"""

import argparse
import json
import os
import random
import subprocess
import sys
import tempfile

import binascii  # MUST be module-level: see docstring env note

BULK_SEED = 0x4E1F


def bulk(idx, n):
    """Deterministic bulk bytes for matrix B.

    random.Random class is not surfaced by antipythony; module-level
    seeded getrandbits is deterministic and present in both runtimes.
    NOTE: byte VALUES match CPython's Mersenne Twister, but this loop is
    O(n) in Python bytecode -- generation cost, not the code under test.
    """
    random.seed(BULK_SEED + idx)
    return bytes(random.getrandbits(8) for _ in range(n))


def build_matrix(full):
    """Yield (case_id, kind, payload) triples. NO closures over loop vars
    (antipythony default-arg capture bug): payloads are plain data."""
    n_bulk = 10000 if full else 120
    cases = []

    # --- A: every length 0..512
    for n in range(513):
        data = bytes((i * 11 + n) & 0xFF for i in range(n))
        cases.append((f"A.enc.{n}", "hexlify", data))
        enc = binascii.hexlify(data)
        if n % 2 == 0:
            cases.append((f"A.dec.{n}", "unhex", enc))
            cases.append((f"A.rt.{n}", "rt", data))
        else:
            cases.append((f"A.decodd.{n}", "unhex", enc))

    # --- B: random lengths to 1 MiB (deterministic)
    for idx in range(n_bulk):
        n = (idx * 2654435761 + 97) % ((1 << 20) + 1)
        cases.append((f"B.enc.{idx}.{n}", "hexlify", bulk(idx, n)))
        cases.append((f"B.dec.{idx}.{n}", "rt", bulk(idx, n)))

    # --- C: non-hex char at every position + SIMD block edges
    for total in (16, 32, 64, 100):
        for pos in range(total):
            t = bytearray(b"a" * total)
            t[pos] = ord("z")
            cases.append((f"C.bad{total}.{pos}", "unhex", bytes(t)))
    for total in (16, 32, 64, 128):
        base_s = b"0123456789abcdef" * (total // 16)
        for pos in (0, 1, 14, 15, 16, 17, 30, 31, 32, 33, 62, 63, 64, 65, 126, 127, 128, 129):
            if pos < total:
                t = bytearray(base_s)
                t[pos] = ord("!")
                cases.append((f"C.edge{total}.{pos}", "unhex", bytes(t)))

    # --- D: case mixes + fromhex whitespace/underscore handling
    hexstrs = [
        "6865", "68 65", " 68 65 ", "68\t65", "68_65", "_68", "68_", "6 8",
        "ABCD", "abcd", "AbCd", "0x68", "GG", "g6", "", "686511", "68651",
        "6", "680065", "68 6", "68 65 11", "68_65_11", "  ", "68_",
    ]
    for i, s in enumerate(hexstrs):
        cases.append((f"D.fromhex.{i}", "fromhex", s))
    cases.append(("D.ba.fromhex", "ba_fromhex", "68 65"))
    for n in (0, 1, 2, 3, 33, 255, 256, 511):
        data = bytes((i * 13 + n) & 0xFF for i in range(n))
        cases.append((f"D.hexupper.{n}", "hexupper", data))
        cases.append((f"D.strhex.{n}", "strhex", data))
        cases.append((f"D.strhex.U.{n}", "strhex_upper", data))

    # --- E: explicit exception-text endpoints + large-input sanity
    cases += [
        ("E.unhex.odd", "unhex", b"abc"),
        ("E.unhex.bad", "unhex", b"zz"),
        ("E.unhex.mid", "unhex", b"abzd"),
        ("E.unhex.0x", "unhex", b"0xab"),
        ("E.fromhex.odd", "fromhex", "68651"),
        ("E.fromhex.bad0", "fromhex", "zz"),
        ("E.fromhex.bad5", "fromhex", "68 65_11"),
        ("E.hexlify.big", "hexlen_big", b""),
    ]
    return cases


def dispatch(kind, payload):
    if kind == "hexlify":
        return binascii.hexlify(payload)
    if kind == "unhex":
        return binascii.unhexlify(payload)
    if kind == "rt":
        return binascii.unhexlify(binascii.hexlify(payload)) == payload
    if kind == "fromhex":
        return bytes.fromhex(payload)
    if kind == "ba_fromhex":
        return bytearray.fromhex(payload)
    if kind == "hexupper":
        return binascii.hexlify(payload).upper()
    if kind == "strhex":
        return payload.hex()
    if kind == "strhex_upper":
        return payload.hex().upper()
    if kind == "hexlen_big":
        return len(binascii.hexlify(bytes(range(256)) * 40))
    raise AssertionError("unknown kind " + kind)


def run_case(fn):
    try:
        return ("ok", repr(fn()))
    except Exception as e:  # noqa: BLE001 - deliberate broad catch
        return ("err", type(e).__name__ + ": " + str(e))


def collect(full):
    out = []
    for cid, kind, payload in build_matrix(full):
        r = run_case(lambda: dispatch(kind, payload))
        out.append([cid, r[0], r[1]])
    return out


def worker_main(full):
    with open(os.environ["DIFF_HEX_OUT"], "w") as f:
        json.dump(collect(full), f)
    return 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--other", nargs="+", default=None,
                    help="interpreter command prefix for the OTHER leg")
    ap.add_argument("--full", action="store_true")
    args = ap.parse_args()

    full = args.full
    if args.other is None:
        other = collect(full)
        other_label = "self"
    else:
        with tempfile.NamedTemporaryFile(suffix=".json", delete=False) as tf:
            out_path = tf.name
        env = dict(os.environ)
        env["DIFF_HEX_WORKER"] = "1"
        env["DIFF_HEX_OUT"] = out_path
        if full:
            env["DIFF_HEX_FULL"] = "1"
        cmd = args.other + [os.path.abspath(__file__)]
        try:
            proc = subprocess.run(cmd, env=env, capture_output=True, text=True, timeout=1800)
        except subprocess.TimeoutExpired:
            print(json.dumps({"probe": "diff_hex", "cases": 0,
                              "mismatches": ["OTHER-TIMEOUT after 1800s"], "rc": 1}))
            return 1
        if proc.returncode != 0:
            print(json.dumps({"probe": "diff_hex", "cases": 0,
                              "mismatches": [f"OTHER-FAILED rc={proc.returncode}",
                                             proc.stderr[-2000:]], "rc": 1}))
            return 1
        with open(out_path) as f:
            other = json.load(f)
        os.unlink(out_path)
        other_label = " ".join(args.other)

    mine = collect(full)

    mismatches = []
    for m, o in zip(mine, other):
        if m != o:
            mismatches.append({"case": m[0], "ref": f"{m[1]} {m[2]}", "other": f"{o[1]} {o[2]}"})
    if len(mine) != len(other):
        mismatches.append({"case": "<length>", "ref": str(len(mine)), "other": str(len(other))})

    summary = {
        "probe": "diff_hex",
        "cases": len(mine),
        "mismatches": mismatches,
        "other": other_label,
        "mode": "full" if full else "smoke",
    }
    print(json.dumps(summary))
    return 1 if mismatches else 0


if __name__ == "__main__":
    if os.environ.get("DIFF_HEX_WORKER") == "1":
        sys.exit(worker_main(os.environ.get("DIFF_HEX_FULL") == "1"))
    sys.exit(main())
