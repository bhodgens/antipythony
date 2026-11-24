#!/usr/bin/env python3
"""diff_b64.py -- byte-exactness differential gate for base64
(leaf 01-base64-simd T1, plans/primitive-perf/01-encode-simd).

Usage:
    python3.14 diff_b64.py --other <cmd...> [--full]   e.g.
    python3.14 diff_b64.py --other /path/to/antipythony run
    (no --other = self mode; the OTHER leg runs this same file as a
    subprocess in WORKER mode via DIFF_B64_WORKER/DIFF_B64_OUT env).

Matrix: (A) binascii b2a/a2b roundtrip every length 0..512; (B) random
lengths to 1 MiB (10k full / 120 smoke, seeded bulk bytes); (C) invalid
byte at EVERY position of 64-byte + 16/32/48/68/100-byte strings; (D)
custom alphabets (binascii newline=, b64encode/b64decode altchars,
urlsafe); (E) validate=/strict_mode= error paths, embedded newlines,
whitespace skipping, non-canonical trailing bits, extra padding.

Env notes (honest, not papered over):
- binascii is imported at module level: antipythony's `import base64`
  does not auto-import binascii (CPython's does); without it the
  antipythony leg mis-executes. Side-effect-free under CPython.
- Cases avoid default-arg capture in loops: antipythony mis-compiles
  `lambda d=data:` there (all lambdas see the last value). Cases are
  (kind, payload) tuples run by a plain dispatcher instead.

Known antipythony divergences this gate flags (gate failures BY DESIGN
until fixed): b64encode altchars ignored; b64decode(altchars) rejects
'-'/'_'; validate error text "Invalid base64-encoded string" vs CPython
"Only base64 data is allowed"/"Incorrect padding"; bare '!' falls to
"Invalid..." instead of "Incorrect padding"; interior space rejected
without validate=; unpadded 'YQ' wrongly decodes.

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

# deterministic bulk generator (fixed seed; no env/time dependence)
SAMPLE_SEED = 0xB64


def bulk(n, salt):
    # random.Random class is not surfaced by antipythony; use the module-level
    # seeded API (getrandbits), which is deterministic and present in both.
    random.seed(SAMPLE_SEED + salt)
    return bytes(random.getrandbits(8) for _ in range(n))


def run_case(fn):
    try:
        return ("ok", repr(fn()))
    except Exception as e:  # noqa: BLE001 - deliberate broad catch
        return ("err", type(e).__name__ + ": " + str(e))


def build_matrix(full):
    """Yield (case_id, kind, payload) triples; dispatcher executes them.

    NO closures over loop variables (antipythony default-arg capture
    bug): payloads are plain data, kinds are plain strings.
    """
    n_bulk = 10000 if full else 120
    cases = []

    # --- A: roundtrip encode+decode for every length 0..512
    for n in range(513):
        data = bytes((i * 7 + n) & 0xFF for i in range(n))
        cases.append((f"A.enc.{n}", "b2a", data))
        enc = binascii.b2a_base64(data, newline=False)
        cases.append((f"A.dec.{n}", "a2b", enc))
        cases.append((f"A.rt.{n}", "rt", data))

    # --- B: bulk random lengths to 1 MiB
    for idx in range(n_bulk):
        n = (idx * 2654435761 + 97) % ((1 << 20) + 1)
        data = bulk(n, idx)
        cases.append((f"B.enc.{idx}.{n}", "b2a", data))
        enc = binascii.b2a_base64(data, newline=False)
        cases.append((f"B.dec.{idx}.{n}", "a2b", enc))

    # --- C: invalid byte at every position
    for total in (16, 32, 48, 64, 68, 100):
        for pos in range(total):
            t = bytearray(b"A" * total)
            t[pos] = 33  # b'!'
            cases.append((f"C.bad{total}.{pos}", "a2b", bytes(t)))

    # --- D: custom alphabets / urlsafe / newline variants (payload = data)
    sample = bytes(range(0, 64, 3)) + b"\xfa\xce\xf0\x0d"
    cases += [
        ("D.b2a.nl_true", "b2a", sample),
        ("D.b2a.nl_false", "b2a", sample),
        ("D.b2a.empty", "b2a", b""),
        ("D.b64enc", "b64enc", sample),
        ("D.b64enc.altchars", "b64enc_alt", sample),
        ("D.b64enc.altchars.ab", "b64enc_alt_ab", sample),
        ("D.urlenc", "urlenc", sample),
        ("D.urldec.roundtrip", "urldec_rt", sample),
        ("D.b64dec.roundtrip", "b64dec_rt", sample),
        ("D.b64dec.altchars.roundtrip", "b64dec_alt_rt", sample),
        ("D.b64dec.altchars.pad", "b64dec_alt_pad", b"-_8"),
        ("D.b64dec.str", "b64dec", "YGVsbG8="),
    ]

    # --- E: validate / strict_mode / newline-embedded / noncanonical / pad
    cases += [
        ("E.validate.ok", "b64dec_v", b"aGVsbG8="),
        ("E.validate.bad", "b64dec_v", b"aGVsbG!="),
        ("E.validate.nonalpha", "b64dec_v", b"aGV!bG8="),
        ("E.nov.nonalpha", "b64dec", b"aGV!"),
        ("E.strict.bad", "a2b_strict", b"aGVsbG8=\n"),
        ("E.strict.ok", "a2b_strict", b"aGVsbG8="),
        ("E.nl_embed", "a2b", b"aGVs\nbG8="),
        ("E.nl_embed.strict", "a2b_strict", b"aGVs\nbG8="),
        ("E.ws.b64dec", "b64dec", b"aGVs bG8=\n"),
        ("E.pad.extra", "b64dec", b"aGVsbG8=="),
        ("E.noncanon.1", "a2b", b"YR=="),
        ("E.noncanon.2", "a2b", b"iI=="),
        ("E.errtext.pad", "b64dec", b"YQ"),
    ]
    return cases


def dispatch(kind, payload):
    if kind == "b2a":
        return binascii.b2a_base64(payload, newline=False)
    if kind == "a2b":
        return binascii.a2b_base64(payload)
    if kind == "rt":
        return binascii.a2b_base64(binascii.b2a_base64(payload, newline=False)) == payload
    if kind == "b64enc":
        return base64.b64encode(payload)
    if kind == "b64enc_alt":
        return base64.b64encode(payload, altchars=b"-_")
    if kind == "b64enc_alt_ab":
        return base64.b64encode(payload, altchars=b"ab")
    if kind == "urlenc":
        return base64.urlsafe_b64encode(payload)
    if kind == "urldec_rt":
        return base64.urlsafe_b64decode(base64.urlsafe_b64encode(payload))
    if kind == "b64dec_rt":
        return base64.b64decode(base64.b64encode(payload))
    if kind == "b64dec_alt_rt":
        return base64.b64decode(base64.b64encode(payload, altchars=b"-_"), altchars=b"-_")
    if kind == "b64dec_alt_pad":
        return base64.b64decode(payload, altchars=b"-_")
    if kind == "b64dec":
        return base64.b64decode(payload)
    if kind == "b64dec_v":
        return base64.b64decode(payload, validate=True)
    if kind == "a2b_strict":
        return binascii.a2b_base64(payload, strict_mode=True)
    raise AssertionError("unknown kind " + kind)


import base64  # noqa: E402  (after dispatch table def for readability)


def collect(full):
    out = []
    for cid, kind, payload in build_matrix(full):
        r = run_case(lambda: dispatch(kind, payload))
        out.append([cid, r[0], r[1]])
    return out


def worker_main(full):
    with open(os.environ["DIFF_B64_OUT"], "w") as f:
        json.dump(collect(full), f)
    return 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--other", nargs="+", default=None,
                    help="interpreter command prefix for the OTHER leg, "
                         "e.g. /path/to/antipythony run")
    ap.add_argument("--full", action="store_true",
                    help="run the full 10k random-length matrix (default smoke: 120)")
    args = ap.parse_args()

    full = args.full
    if args.other is None:
        other = collect(full)
        other_label = "self"
    else:
        with tempfile.NamedTemporaryFile(suffix=".json", delete=False) as tf:
            out_path = tf.name
        env = dict(os.environ)
        env["DIFF_B64_WORKER"] = "1"
        env["DIFF_B64_OUT"] = out_path
        if full:
            env["DIFF_B64_FULL"] = "1"
        cmd = args.other + [os.path.abspath(__file__)]
        try:
            proc = subprocess.run(cmd, env=env, capture_output=True, text=True, timeout=1800)
        except subprocess.TimeoutExpired:
            print(json.dumps({"probe": "diff_b64", "cases": 0,
                              "mismatches": ["OTHER-TIMEOUT after 1800s"], "rc": 1}))
            return 1
        if proc.returncode != 0:
            print(json.dumps({"probe": "diff_b64", "cases": 0,
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
        "probe": "diff_b64",
        "cases": len(mine),
        "mismatches": mismatches,
        "other": other_label,
        "mode": "full" if full else "smoke",
    }
    print(json.dumps(summary))
    return 1 if mismatches else 0


if __name__ == "__main__":
    if os.environ.get("DIFF_B64_WORKER") == "1":
        sys.exit(worker_main(os.environ.get("DIFF_B64_FULL") == "1"))
    sys.exit(main())
