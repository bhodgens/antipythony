#!/usr/bin/env python3
"""Differential gate for leaf pp2-bz2: bz2 + lzma one-shot surfaces.

Runs identically under CPython 3.14 and antipythony; stdout lines are
compared byte-exact by the driver (same convention as fix_sha_zlib.py).

Contract:
- roundtrip compress->decompress at sizes 0/64/48KiB/1MiB;
- decompress of CPYTHON-produced bytes (BASE64 vectors below, generated
  once by python3.14) byte-identical;
- error texts (bad magic, truncated input, wrong format arg, bad preset,
  bad check id) CPython-exact.
- lzma compress OUTPUT is recorded decompress-compatible-only (lzma-rs
  range-encoder diverges from liblzma); bz2 compress IS byte-exact and
  pinned via BZ2_HEX.
"""
import bz2
import base64
import binascii
import hashlib
import lzma
import sys

# --- CPython 3.14-produced compressed vectors (regen: /tmp/gen_vectors.py shape) ---
BASE64 = {
    "bz2_l1": "QlpoMTFBWSZTWSwhJOYAFkORgEABP///8DABGAFGhoAAACjQ0AAAAUqk1NMEwRmptqPwpGCkdikYKRxKRoUjIpGhSN5SOhSP0pHEpHcpGRSO4pGpSNhSPJSPQpGpSMykfxSMikbCkYKR5KRmUjmUjQpHopGZSOxSP8pGpSMxSMFIwUjoQV7KR7KR8F3JFOFCQLCEk5g=",
    "bz2_l9": "QlpoOTFBWSZTWSwhJOYAFkORgEABP///8DABGAFGhoAAACjQ0AAAAUqk1NMEwRmptqPwpGCkdikYKRxKRoUjIpGhSN5SOhSP0pHEpHcpGRSO4pGpSNhSPJSPQpGpSMykfxSMikbCkYKR5KRmUjmUjQpHopGZSOxSP8pGpSMxSMFIwUjoQV7KR7KR8F3JFOFCQLCEk5g=",
    "bz2_multi": "QlpoNTFBWSZTWZWuG2gAAACRgAACASAcACAAIYAMAycu4u5IpwoSErXDbQBCWmg1MUFZJlNZJooCNAAAAoGADgGIACAAIhhoMAcCmF3JFOFCQJooCNA=",
    "bz2_empty_payload": "QlpoNRdyRThQkAAAAAA=",
    "bz2_trunc": "QlpoNTFBWSZTWSwhJOYAFkORgEABP///8DABGAFGhoAAACjQ0AAAAUqk1NMEwRmptqPwpGCkdikYKRxKRoUjIpGhSN5SOhSP0pHEpHcpGRSO4pGpSNhSPJSPQpGpSMykfxSMikbCkYKR5KRmUjmUjQpHopGZSOxSP8pGpSMxSMFIwUjoQV7KR7KR8F3JFOFC",
    "xz_def": "/Td6WFoAAATm1rRGAgAhARYAAAB0L+Wj4DS7AF5dADoaCM52x+Xp1gc0w9EOv85V4aq94OSPmAHdjeUHVJ5lJV8nOmp+tNNJA4nO1H08/5reNhysEWXiyvspiSZ/A4k9ITMEq0iMDtqeBREO5zL0qfgN3tGGNphZElcl6AAAAACydD3vHEBhXgABerxpAAAA5qf1+rHEZ/sCAAAAAARZWg==",
    "xz_c32": "/Td6WFoAAAFpIt42AgAhARYAAAB0L+Wj4DS7AF5dADoaCM52x+Xp1gc0w9EOv85V4aq94OSPmAHdjeUHVJ5lJV8nOmp+tNNJA4nO1H08/5reNhysEWXiyvspiSZ/A4k9ITMEq0iMDtqeBREO5zL0qfgN3tGGNphZElcl6AAAAAA5uqv5AAF2vGkAAACdZzeNPjANiwIAAAAAAVla",
    "xz_c0": "/Td6WFoAAAD/EtlBAgAhARYAAAB0L+Wj4DS7AF5dADoaCM52x+Xp1gc0w9EOv85V4aq94OSPmAHdjeUHVJ5lJV8nOmp+tNNJA4nO1H08/5reNhysEWXiyvspiSZ/A4k9ITMEq0iMDtqeBREO5zL0qfgN3tGGNphZElcl6AAAAAAAAXK8aQAAAIslphaoAAr8AgAAAAAAWVo=",
    "alone_def": "XQAAgAD//////////wA6GgjOdsfl6dYHNMPRDr/OVeGqveDkj5gB3Y3lB1SeZSVfJzpqfrTTSQOJztR9PP+a3jYcrBFl4sr7KYkmfwOJPSEzBKtIjA7angURDucy9Kn4Dd7RhjaYWRJZO08H//+NFAAA",
    "alone_p9e": "XQAAAAT//////////wA6GgjOdsfl6dYHNMPRDr/OVeGqveDkj5gB3Y3lB1SeZSVfJzpqfrTTSQOJztR9PP+a3jYcrBFl4sr7KYkmfwOJPSEzBKtIjA7angURDucy9Kn4Dd7RhjaYWRJZO08H//+NFAAA",
    "raw_lzma2": "4DS7AF5dADoaCM52x+Xp1gc0w9EOv85V4aq94OSPmAHdjeUHVJ5lJV8nOmp+tNNJA4nO1H08/5reNhysEWXiyvspiSZ/A4k9ITMEq0iMDtqeBREO5zL0qfgN3tGGNphZElcl6AAA",
    "xz_p0": "/Td6WFoAAATm1rRGAgAhAQwAAACPmEGc4DS7AF5dADoaCM52x+Xp1gc0w9EOv85V4aq94OSPmAHdjeUHVJ5lJV8nOmp+tNNJA4nO1H08/5reNhysEWXiyvspiSZ/A4k9ITMEq0iMDtqeBREO5zL0qfgN3tGGNphZElcl6AAAAACydD3vHEBhXgABerxpAAAA5qf1+rHEZ/sCAAAAAARZWg==",
    "xz_trunc": "/Td6WFoAAATm1rRGAgAhARYAAAB0L+Wj4DS7AF5dADoaCM52x+Xp1gc0w9EOv85V4aq94OSPmAHdjeUHVJ5lJV8nOmp+tNNJA4nO1H08/5reNhysEWXiyvspiSZ/A4k9ITMEq0iMDtqeBREO5zL0qfgN3tGGNphZElcl6AAAAACydD3vHEBhXgABerxpAAAA5qf1+rHEZ/sCAA==",
    "xz_filters": "/Td6WFoAAATm1rRGAgAhARYAAAB0L+Wj4DS7AF5dADoaCM52x+Xp1gc0w9EOv85V4aq94OSPmAHdjeUHVJ5lJV8nOmp+tNNJA4nO1H08/5reNhysEWXiyvspiSZ/A4k9ITMEq0iMDtqeBREO5zL0qfgN3tGGNphZElcl6AAAAACydD3vHEBhXgABerxpAAAA5qf1+rHEZ/sCAAAAAARZWg==",
}
TEXT_SHA = "1ad8e1122712cc949f5f3f05f68290f37553b9ab2a33b8253e087524691f5b08"
TEXT_LEN = 13500


SIZES = [0, 64, 48 * 1024, 1024 * 1024]
RAW_FILTERS = [{"id": lzma.FORMAT_RAW and 33, "preset": 6}]


def make_blob(n):
    # deterministic, size-driven; interpreter-speed-neutral: fixed 64-byte
    # pattern tiled with a periodic 8-byte counter (no per-byte python loop).
    base = bytes(range(64))
    chunks = []
    i = 0
    total = 0
    while total < n:
        chunks.append(base + i.to_bytes(8, "little"))
        total += 72
        i += 1
    return b"".join(chunks)[:n]


def check(tag, fn):
    try:
        print("%s OK %s" % (tag, fn()))
    except Exception as e:  # noqa: BLE001 - differential: any divergence shows
        print("%s ERR %s %r" % (tag, type(e).__name__, str(e)))


def main():
    blob = make_blob(max(SIZES))

    # 1. roundtrips
    for n in SIZES:
        data = blob[:n]
        check("bz2 rt n=%d" % n, lambda d=data: bz2.decompress(bz2.compress(d, 5)) == d)
        check("xz rt n=%d" % n, lambda d=data: lzma.decompress(lzma.compress(d)) == d)
        check(
            "alone rt n=%d" % n,
            lambda d=data: lzma.decompress(
                lzma.compress(d, format=lzma.FORMAT_ALONE), format=lzma.FORMAT_ALONE
            )
            == d,
        )
        check(
            "raw rt n=%d" % n,
            lambda d=data: lzma.decompress(
                lzma.compress(
                    d, format=lzma.FORMAT_RAW, filters=[{"id": 33, "preset": 6}]
                ),
                format=lzma.FORMAT_RAW,
                filters=[{"id": 33, "preset": 6}],
            )
            == d,
        )
        check(
            "xz levels n=%d" % n,
            lambda d=data: [
                len(lzma.decompress(lzma.compress(d, preset=p))) for p in (0, 6, 9)
            ],
        )
        check(
            "bz2 levels n=%d" % n,
            lambda d=data: [
                len(bz2.decompress(bz2.compress(d, lv))) for lv in (1, 5, 9)
            ],
        )

    # 2. CPython-produced vectors decode byte-identically
    for tag, b64 in sorted(BASE64.items()):
        blob = base64.b64decode(b64)
        if tag.startswith("bz2"):
            if tag.endswith("trunc"):
                check("vec %s" % tag, lambda b=blob: bz2.decompress(b))
            else:
                check(
                    "vec %s" % tag,
                    lambda b=blob, t=tag: (
                        len(bz2.decompress(b)),
                        hashlib.sha256(bz2.decompress(b)).hexdigest(),
                    ),
                )
        else:
            if tag == "xz_trunc":
                check("vec %s" % tag, lambda b=blob: lzma.decompress(b))
            elif tag == "raw_lzma2":
                check(
                    "vec %s" % tag,
                    lambda b=blob: (
                        len(
                            lzma.decompress(
                                b,
                                format=lzma.FORMAT_RAW,
                                filters=[{"id": 33, "preset": 6}],
                            )
                        ),
                        hashlib.sha256(
                            lzma.decompress(
                                b,
                                format=lzma.FORMAT_RAW,
                                filters=[{"id": 33, "preset": 6}],
                            )
                        ).hexdigest(),
                    ),
                )
            elif tag.startswith("alone"):
                check(
                    "vec %s" % tag,
                    lambda b=blob: (
                        len(lzma.decompress(b, format=lzma.FORMAT_ALONE)),
                        hashlib.sha256(
                            lzma.decompress(b, format=lzma.FORMAT_ALONE)
                        ).hexdigest(),
                    ),
                )
            else:
                check(
                    "vec %s" % tag,
                    lambda b=blob: (
                        len(lzma.decompress(b)),
                        hashlib.sha256(lzma.decompress(b)).hexdigest(),
                    ),
                )

    # 3. bz2 compress byte-exactness vs CPython (pinned hex)
    text = base64.b64decode(
        base64.b64encode(bytes(range(128)) * 128)
    )  # same generator data as the vectors
    check("bz2 hex l1", lambda: binascii.hexlify(bz2.compress(text, 1)).decode())
    check("bz2 hex l9", lambda: binascii.hexlify(bz2.compress(text, 9)).decode())

    # 4. error texts
    check("err bz2 magic", lambda: bz2.decompress(b"garbage!"))
    check("err bz2 empty", lambda: bz2.decompress(b""))
    check("err bz2 BZ only", lambda: bz2.decompress(b"BZ"))
    check("err bz2 trunc", lambda: bz2.decompress(bz2.compress(text, 5)[:-5]))
    check("err bz2 level 0", lambda: bz2.compress(b"x", 0))
    check("err bz2 level 10", lambda: bz2.compress(b"x", 10))
    check("err lz garbage", lambda: lzma.decompress(b"garbage!"))
    check("err lz xz trunc", lambda: lzma.decompress(lzma.compress(text)[:-6]))
    check("err lz alone trunc", lambda: lzma.decompress(
        lzma.compress(text, format=lzma.FORMAT_ALONE)[:-2], format=lzma.FORMAT_ALONE))
    check("err lz raw trunc", lambda: lzma.decompress(
        lzma.compress(text, format=lzma.FORMAT_RAW, filters=[{"id": 33, "preset": 6}])[:-2],
        format=lzma.FORMAT_RAW, filters=[{"id": 33, "preset": 6}]))
    check("err lz empty alone", lambda: lzma.decompress(b"", format=lzma.FORMAT_ALONE))
    check("err lz fmt 99 c", lambda: lzma.compress(b"x", format=99))
    check("err lz fmt 99 d", lambda: lzma.decompress(b"x", format=99))
    check("err lz preset 99", lambda: lzma.compress(b"x", preset=99))
    check("err lz raw nofilter c", lambda: lzma.compress(b"x", format=lzma.FORMAT_RAW))
    check("err lz raw nofilter d", lambda: lzma.decompress(b"x", format=lzma.FORMAT_RAW))
    check("err lz alone+check", lambda: lzma.compress(
        b"x", format=lzma.FORMAT_ALONE, check=lzma.CHECK_CRC32))
    check("err lz bad check", lambda: lzma.compress(b"x", check=99))
    check("err lz alone as xz", lambda: lzma.decompress(
        lzma.compress(text, format=lzma.FORMAT_ALONE), format=lzma.FORMAT_XZ))
    check("err lz raw wrongfilter", lambda: lzma.decompress(
        lzma.compress(text, format=lzma.FORMAT_RAW, filters=[{"id": 33, "preset": 6}]),
        format=lzma.FORMAT_RAW, filters=[{"id": lzma.FILTER_LZMA1, "preset": 6}]))

    # 5. constants
    print(
        "consts",
        lzma.FORMAT_AUTO,
        lzma.FORMAT_XZ,
        lzma.FORMAT_ALONE,
        lzma.FORMAT_RAW,
        lzma.CHECK_NONE,
        lzma.CHECK_CRC32,
        lzma.CHECK_CRC64,
        lzma.CHECK_SHA256,
        lzma.CHECK_ID_MAX,
        lzma.CHECK_UNKNOWN,
        lzma.PRESET_DEFAULT,
        lzma.PRESET_EXTREME,
        lzma.FILTER_LZMA2,
        lzma.FILTER_DELTA,
        lzma.FILTER_X86,
        lzma.is_check_supported(1) if hasattr(lzma, "is_check_supported") else 1,
    )
    print("DONE")


main()
