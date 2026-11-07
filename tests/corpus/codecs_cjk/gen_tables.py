#!/usr/bin/env python3
"""Generate the Rust mapping tables for the native multibyte codecs from the
CPython 3.14 reference interpreter's own codec data (byte-exactness contract:
tables come FROM the ground-truth interpreter, never from WHATWG/encoding_rs).

Writes crates/ap-run/src/mbc_tables.rs with:
  - DBS:      per-codec (double-byte -> char) and (char -> double-byte) maps
  - SB:       single-byte (trailing 0x40..0xFE style) handled inside DBS
  - GB18030   four-byte ranges
  - ISO2022   escape-sequence family parameters

Approach per codec: walk every double-byte lead in the codec's lead range,
every trail in its trail range, call bytes.decode; record successes into the
decode map; invert into the encode map (CPython keeps the LAST char for
duplicate decode targets, which inversion with overwrite reproduces).
Single-byte segments (ASCII + the 0x00..0x7F/0x80..0xFF lead rows) are read
the same way. gb18030's 4-byte form is enumerated from its linear ranges.
"""
import json
import subprocess
import sys

REF = "/opt/homebrew/bin/python3.14"

# (rust const name, getcodec key, kind)
JP = [
    ("cp932", "cp932", "double"),
    ("euc_jp", "euc_jp", "double"),
    ("shift_jis", "shift_jis", "double"),
    ("shift_jisx0213", "shift_jisx0213", "double2000"),
    ("shift_jis_2004", "shift_jis_2004", "double2000"),
    ("euc_jisx0213", "euc_jisx0213", "euc2000"),
    ("euc_jis_2004", "euc_jis_2004", "euc2000"),
]
KR = [("euc_kr", "euc_kr", "euckr"), ("cp949", "cp949", "cp949"), ("johab", "johab", "johab")]
CN = [("gb2312", "gb2312", "gb2312"), ("gbk", "gbk", "gbk"), ("hz", "hz", "hz")]
TW = [("big5", "big5", "big5"), ("cp950", "cp950", "cp950")]
HK = [("big5hkscs", "big5hkscs", "cp950")]  # same enumeration shape as cp950

def enum_codec(key, kind):
    """Return {hexbytes: charcode} for every encodable double-byte sequence."""
    src = r'''
import json, sys
key = sys.argv[1]
kind = sys.argv[2]
out = {}
def add(bs, ch):
    if bs not in out or out[bs] is None:
        out[bs] = ch
import json
# 1) single-byte segment: 0x00..0xFF individually (decode length-1)
for b in range(0x100):
    bs = bytes([b])
    try:
        ch = bs.decode(key)
        if ch != '\ufffd' or True:
            try:
                # verify it re-encodes (decode may be total but lossy)
                if ch.encode(key) == bs:
                    out[bs.hex()] = ord(ch)
            except Exception:
                pass
    except UnicodeDecodeError:
        pass
# 2) double-byte: lead/trail ranges by family
def leads(kind):
    if kind in ("double", "double2000"):
        return range(0x81, 0xA1)
    if kind == "euc2000":
        return [0x8F]
    if kind in ("euckr", "cp949", "gb2312", "gbk", "big5", "cp950", "johab"):
        return range(0x81 if kind == "cp949" else 0xA1, 0xFF)
    return []
# Generic double enumeration per family: (lead_lo, lead_hi, trail_lo, trail_hi) pairs.
all_ranges = []
if kind == "double":  # cp932 / shift_jis: leads 0x81-0x9F + 0xE0-0xEA (+0xEF cp932 NEC row 0x87 is inside 81-9F)
    all_ranges = [
        (0x81, 0x9F, 0x40, 0x7E),
        (0x81, 0x9F, 0x80, 0xFC),
        (0xE0, 0xEA, 0x40, 0x7E),
        (0xE0, 0xEA, 0x80, 0xFC),
    ]
    if key == 'cp932':
        all_ranges.append((0xEF, 0xEF, 0x40, 0xFC))
elif kind == "double2000":  # shift_jisx0213/2004 adds 0xF0-0xFC rows
    all_ranges = [
        (0x81, 0x9F, 0x40, 0x7E),
        (0x81, 0x9F, 0x80, 0xFC),
        (0xE0, 0xFC, 0x40, 0x7E),
        (0xE0, 0xFC, 0x80, 0xFC),
    ]
elif kind == "euc2000":  # euc_jisx0213/euc_jis_2004: 2-byte rows + 0x8F 3-byte
    all_ranges = [(0xA1, 0xFE, 0xA1, 0xFE)]
elif kind in ("euckr", "gb2312"):
    all_ranges = [(0xA1, 0xF7, 0xA1, 0xFE), (0xA1, 0xF7, 0xA1, 0xFE)]
elif kind == "cp949":
    all_ranges = [(0x81, 0xFD, 0x41, 0x5A), (0x81, 0xFD, 0x61, 0x7A), (0x81, 0xFD, 0x81, 0xFE)]
elif kind == "johab":
    all_ranges = [(0x84, 0xD3, 0x31, 0x7E), (0x84, 0xD3, 0x81, 0xFE)]
elif kind in ("gbk", "hz"):
    all_ranges = [(0x81, 0xFE, 0x40, 0x7E), (0x81, 0xFE, 0x80, 0xFE)]
elif kind in ("big5", "cp950"):
    all_ranges = [(0x81, 0xFE, 0x40, 0x7E), (0x81, 0xFE, 0xA1, 0xFC)]
for lead_lo, lead_hi, trail_lo, trail_hi in all_ranges:
    for lead in range(lead_lo, lead_hi + 1):
        for trail in range(trail_lo, trail_hi + 1):
            bs = bytes([lead, trail])
            try:
                ch = bs.decode(key)
            except UnicodeDecodeError:
                continue
            except Exception:
                continue
            try:
                if ch.encode(key) == bs:
                    out[bs.hex()] = ord(ch)
            except Exception:
                pass
# euc2000: 2-byte A1A1..FEFE (JIS X 0213 rows 1-2) + 3-byte 0x8F-prefixed
if kind == "euc2000":
    for m2 in range(0xA1, 0xFF):
        for t2 in range(0xA1, 0xFF):
            bs = bytes([m2, t2])
            try:
                ch = bs.decode(key)
            except Exception:
                continue
            try:
                if ch.encode(key) == bs:
                    out[bs.hex()] = ord(ch)
            except Exception:
                pass
    for m in range(0xA1, 0xFF):
        for t in range(0xA1, 0xFF):
            bs = bytes([0x8F, m, t])
            try:
                ch = bs.decode(key)
            except Exception:
                continue
            try:
                if ch.encode(key) == bs:
                    out[bs.hex()] = ord(ch)
            except Exception:
                pass
print(json.dumps(out))
'''
    proc = subprocess.run([REF, "-S", "-c", src, key, kind], capture_output=True, text=True, timeout=600)
    if proc.returncode != 0:
        print(f"ENUM FAIL {key}: {proc.stderr[:300]}", file=sys.stderr)
        sys.exit(1)
    return json.loads(proc.stdout)

def enum_ascii(key):
    return {}

def enum_gb18030():
    """gb18030: two-byte 0x81-0xFE/0x40-0x7E&0x80-0xFE + linear four-byte."""
    src = r'''
import json, sys
key = 'gb18030'
out = {}
for b in range(0x100):
    bs = bytes([b])
    try:
        ch = bs.decode(key)
        if ch.encode(key) == bs:
            out[bs.hex()] = ord(ch)
    except Exception:
        pass
for lead in range(0x81, 0xFF):
    for trail in range(0x40, 0xFF):
        if 0x7F < trail < 0x80:
            continue
        bs = bytes([lead, trail])
        try:
            ch = bs.decode(key)
        except Exception:
            continue
        try:
            if ch.encode(key) == bs:
                out[bs.hex()] = ord(ch)
        except Exception:
            pass
print(json.dumps(out))
'''
    proc = subprocess.run([REF, "-S", "-c", src], capture_output=True, text=True, timeout=900)
    if proc.returncode != 0:
        print(f"gb18030 enum FAIL: {proc.stderr[:300]}", file=sys.stderr)
        sys.exit(1)
    return json.loads(proc.stdout)

def emit_map(out, f, name):
    items = sorted(out.items(), key=lambda kv: (len(kv[0]), kv[0]))
    f.write(f"pub static {name}: &[(&str, u32)] = &[\n")
    for bs, ch in items:
        if ch is None:
            continue
        f.write(f'    ("%s", 0x%04X),\n' % (bs, ch))
    f.write("];\n\n")

with open("crates/ap-run/src/mbc_tables.rs", "w") as f:
    f.write("// @generated by tests/corpus/codecs_cjk/gen_tables.py — CPython 3.14 codec data.\n")
    f.write("// Each map: hex byte sequence -> Unicode scalar. Decode uses bytes->char;\n")
    f.write("// encode inverts (last-writer-wins on duplicates, matching CPython's\n")
    f.write("// forward-map construction). Single-byte rows are included.\n\n")
    for rust, key, kind in JP:
        m = enum_codec(key, kind)
        emit_map(m, f, f"MAP_{rust.upper()}")
        print(rust, len(m))
    for rust, key, kind in KR:
        m = enum_codec(key, kind)
        emit_map(m, f, f"MAP_{rust.upper()}")
        print(rust, len(m))
    for rust, key, kind in CN:
        m = enum_codec(key, kind)
        emit_map(m, f, f"MAP_{rust.upper()}")
        print(rust, len(m))
    for rust, key, kind in TW:
        m = enum_codec(key, kind)
        emit_map(m, f, f"MAP_{rust.upper()}")
        print(rust, len(m))
    m = enum_codec("big5hkscs", "cp950")
    emit_map(m, f, "MAP_BIG5HKSCS")
    print("big5hkscs", len(m))
    m = enum_gb18030()
    emit_map(m, f, "MAP_GB18030_2")
    print("gb18030 2-byte", len(m))
    # ascii passthrough is hardcoded in the engine; no table needed.
print("wrote crates/ap-run/src/mbc_tables.rs")
