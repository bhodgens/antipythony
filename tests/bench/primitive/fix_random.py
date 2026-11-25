#!/usr/bin/env python3
"""fix_random.py -- differential probe gate for leaf ppfix-random
(random.Random class on the native random module).

Usage:
    python3.14 fix_random.py --other <cmd...>   e.g.
    python3.14 fix_random.py --other target-ppfix-random/release/antipythony run
    (no --other = self mode under CPython; the OTHER leg runs this same file
    as a subprocess and both outputs are compared).

Gates (a) from the leaf contract:
- construction with fixed seeds; .random() x1000 sequence self-consistency
  (same seed -> same sequence across two instances AND vs module-level
  seed()+random());
- CPython-exact sequence parity where the MT19937 plumbing falls out free
  (getrandbits 8/32/33/63/64, randrange, randint, sample, choice draws);
- randrange/randint boundary + error texts (exact CPython ValueError text);
- choice/shuffle/sample semantics (types/lengths/ranges/invariants:
  sample length k with no dup indices, shuffle is a permutation).

Exit 0 = green, 1 = mismatch(es). Prints one JSON summary line.
"""
import argparse
import json
import subprocess
import sys
import random


def check(name, cond):
    if not cond:
        raise AssertionError(name)


def run_body(tag):
    """The probe body; every failure raises with the case tag."""
    out = {}

    # --- construction + .random() x1000 self-consistency -----------------
    r1 = random.Random(42)
    r2 = random.Random(42)
    seq1 = [r1.random() for _ in range(1000)]
    seq2 = [r2.random() for _ in range(1000)]
    check(f"{tag}:inst-selfconsistency", seq1 == seq2)

    random.seed(42)
    seq3 = [random.random() for _ in range(1000)]
    check(f"{tag}:inst-vs-module-random", seq1 == seq3)

    # negative big seed = abs (CPython random_seed negation -> abs)
    check(f"{tag}:neg-seed-abs",
          random.Random(-42).random() == random.Random(42).random())

    # CPython-exact getrandbits parity (32/33/63/64 words)
    g = random.Random(0xC0FFEE)
    check(f"{tag}:getrandbits32",
          [g.getrandbits(32) for _ in range(2)] == [4116735509, 2500757463])
    g = random.Random(42)
    check(f"{tag}:getrandbits33",
          [g.getrandbits(33) for _ in range(2)] == [2746317213, 4402387665])
    g = random.Random(0xC0FFEE)
    check(f"{tag}:getrandbits63",
          g.getrandbits(63) == 5370335761375716885)
    g = random.Random(0xC0FFEE)
    check(f"{tag}:getrandbits64",
          g.getrandbits(64) == 10740671522929665557)

    # randrange / randint exact draws (probed on CPython 3.14)
    rr = random.Random(42)
    check(f"{tag}:randrange-62bit",
          [rr.randrange(0, 1 << 62) for _ in range(3)]
          == [1026847926404610461, 2258728696626565719, 1287010195568088798])
    check(f"{tag}:randrange-c0ffee",
          random.Random(0xC0FFEE).randrange(0, 1 << 62) == 1598940245907959032)
    check(f"{tag}:randint",
          random.Random(42).randint(1, 6) == 6)
    check(f"{tag}:randrange-step",
          random.Random(7).randrange(10, 100, 7) == 45)

    # sample / choice exact draws
    check(f"{tag}:sample", random.Random(0xC0FFEE).sample(range(10), 3) == [9, 2, 0])
    check(f"{tag}:choice", random.Random(5).choice([7, 8, 9]) == 9)

    # uniform range invariant
    u = random.Random(3).uniform(1.0, 5.0)
    check(f"{tag}:uniform-range", 1.0 <= u <= 5.0)

    # shuffle is a permutation; same seed -> same permutation (self-consistency)
    l1 = list(range(64))
    l2 = list(range(64))
    random.Random(11).shuffle(l1)
    random.Random(11).shuffle(l2)
    check(f"{tag}:shuffle-permutation", sorted(l1) == list(range(64)))
    check(f"{tag}:shuffle-deterministic", l1 == l2)

    # sample invariants over many (seed, k)
    for seed in range(10):
        pop = range(50)
        s = random.Random(seed).sample(pop, 7)
        check(f"{tag}:sample-len-{seed}", len(s) == 7)
        check(f"{tag}:sample-nodup-{seed}", len(set(s)) == 7)
        check(f"{tag}:sample-range-{seed}", set(s) <= set(pop))

    # VERSION attr
    check(f"{tag}:version", random.Random(1).VERSION == 3)

    # type name
    check(f"{tag}:typename", type(random.Random(1)).__name__ == "Random")

    # --- error texts (exact CPython ValueError/TypeError/IndexError) -----
    def expect(exc, fn, text):
        try:
            fn()
        except exc as e:
            check(f"{tag}:errtext-{text}", str(e) == text)
        else:
            check(f"{tag}:erraised-{text}", False)

    r = random.Random(1)
    expect(ValueError, lambda: r.randrange(5, 3), "empty range in randrange(5, 3)")
    expect(ValueError, lambda: r.randrange(0), "empty range for randrange()")
    expect(ValueError, lambda: r.randrange(-5), "empty range for randrange()")
    expect(ValueError, lambda: r.randint(5, 3), "empty range in randint(5, 3)")
    expect(ValueError, lambda: r.randrange(1, 10, 0), "zero step for randrange()")
    expect(IndexError, lambda: r.choice([]), "Cannot choose from an empty sequence")
    expect(ValueError, lambda: r.sample([1, 2], 3),
           "Sample larger than population or is negative")
    expect(ValueError, lambda: r.sample([1, 2], -1),
           "Sample larger than population or is negative")
    expect(ValueError, lambda: r.getrandbits(-1), "Cannot convert negative int")
    out["ok"] = True
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--other", nargs=argparse.REMAINDER, default=None)
    args = ap.parse_args()

    if args.other:
        cmd = args.other + [__file__]
        cp = subprocess.run(cmd, capture_output=True, text=True, timeout=120)
        their = None
        for line in cp.stdout.splitlines():
            line = line.strip()
            if line.startswith("{"):
                try:
                    their = json.loads(line)
                except json.JSONDecodeError:
                    pass
        mine = run_body("self")
        result = {
            "probe": "fix_random",
            "other_rc": cp.returncode,
            "self_ok": mine.get("ok", False),
            "other_ok": bool(their and their.get("ok")),
            "pass": cp.returncode == 0 and bool(their and their.get("ok")),
        }
    else:
        run_body("other")
        print(json.dumps({"probe": "fix_random", "ok": True}))
        return 0

    print(json.dumps(result))
    return 0 if result["pass"] else 1


if __name__ == "__main__":
    sys.exit(main())
