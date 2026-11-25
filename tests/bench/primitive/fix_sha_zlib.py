#!/usr/bin/env python3
"""Differential probe for leaf ppfix-perf: zlib.decompress + hashlib.

Runs identically under CPython 3.14 and antipythony; stdout lines are
compared byte-exact by the driver.

Input: the driver pre-generates DATAFILE (seeded, identical for both sides)
to avoid interpreter loop-speed differences polluting the matrix.
Sizes 0..64, 1KiB, 48KiB, 1MiB slices; zlib roundtrip at levels 1/6 with
wbits 15 / -15 / 47(gzip); hashlib digests for every native algorithm;
corrupt-data / truncated / garbage exception texts; wbits error text.
"""
import os
import sys
import zlib
import hashlib

DATAFILE = (
    sys.argv[1]
    if len(sys.argv) > 1
    else os.path.join(os.path.dirname(os.path.abspath(__file__)), "pfz_data.bin")
)


def digest_line(tag, data):
    algos = [
        ("md5", hashlib.md5), ("sha1", hashlib.sha1),
        ("sha224", hashlib.sha224), ("sha256", hashlib.sha256),
        ("sha384", hashlib.sha384), ("sha512", hashlib.sha512),
        ("sha3_256", hashlib.sha3_256), ("blake2b", hashlib.blake2b),
        ("blake2s", hashlib.blake2s),
    ]
    parts = []
    for name, fn in algos:
        parts.append("%s=%s" % (name, fn(data).hexdigest()))
    print("%s | %s" % (tag, " ".join(parts)))


def dec_line(tag, compressed, wbits=None):
    try:
        if wbits is None:
            out = zlib.decompress(compressed)
        else:
            out = zlib.decompress(compressed, wbits)
        print("%s OK len=%d sha=%s" % (tag, len(out), hashlib.sha256(out).hexdigest()))
    except zlib.error as e:
        print("%s ZLIBERR %r" % (tag, str(e)))
    except Exception as e:  # noqa: BLE001 - differential: any divergence shows
        print("%s OTHERERR %s %r" % (tag, type(e).__name__, str(e)))


def main():
    with open(DATAFILE, "rb") as f:
        blob = f.read()
    sizes = list(range(0, 65)) + [1024, 48 * 1024, 1024 * 1024]
    for n in sizes:
        data = blob[:n]
        digest_line("RAW n=%d" % n, data)
        for level in (1, 6):
            z = zlib.compress(data, level)
            dec_line("Z n=%d lvl=%d" % (n, level), z)
            raw = z[2:-4]  # strip zlib header+adler -> raw deflate
            dec_line("RAWDEC n=%d lvl=%d" % (n, level), raw, -15)
            # corrupt: flip a byte mid-stream
            if len(z) > 4:
                bad = bytearray(z)
                bad[len(bad) // 2] ^= 0xFF
                dec_line("CORRUPT n=%d lvl=%d" % (n, level), bytes(bad))
            # truncated stream
            if len(z) > 6:
                dec_line("TRUNC n=%d lvl=%d" % (n, level), z[: len(z) - 3])
            # garbage (not a zlib stream at all)
            dec_line("GARBAGE n=%d lvl=%d" % (n, level), data[: min(n, 16)])
        # gzip container (wbits 25..47 arm)
        try:
            gz = zlib.compress(data, 6, 31)
            dec_line("GZ n=%d" % n, gz, 47)
        except Exception as e:  # noqa: BLE001
            print("GZ n=%d MAKEERR %s %r" % (n, type(e).__name__, str(e)))
    # wbits error text (wbits=1 is invalid for decompress)
    data = blob[:64]
    dec_line("WBITS-BAD", zlib.compress(data, 6), 1)
    print("DONE")


if __name__ == "__main__":
    main()
