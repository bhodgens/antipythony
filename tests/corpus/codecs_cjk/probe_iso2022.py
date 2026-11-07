#!/usr/bin/env python3
"""Probe CPython ISO-2022 codecs: which designations are accepted, and what
charset each switches to (probe with a candidate payload and see if it decodes)."""
import json
import subprocess
import sys

REF = "/opt/homebrew/bin/python3.14"

src = r'''
import json, itertools
KEYS = ['iso2022_jp', 'iso2022_jp_1', 'iso2022_jp_2', 'iso2022_jp_3',
        'iso2022_jp_ext', 'iso2022_jp_2004', 'iso2022_kr']
# candidate ESC designations (the standard 94-char multibyte set forms)
DESIGS = [
    b'\x1b$B', b'\x1b$@', b'\x1b$A', b'\x1b$)C', b'\x1b$)A', b'\x1b$)G',
    b'\x1b$(B', b'\x1b$(C', b'\x1b$(D', b'\x1b$(O', b'\x1b$(P', b'\x1b$(Q',
    b'\x1b$(H', b'\x1b$(I', b'\x1b$(J', b'\x1b$)E', b'\x1b$)F', b'\x1b$)G',
    b'\x1b$)H', b'\x1b$)I', b'\x1b$)J', b'\x1b$*H', b'\x1b$+H',
    b'\x1b(B', b'\x1b(J',
]
# payloads per charset guess: use a known char for each table.
# JIS X 0208 row1 = 2121 (iden space)... use distinctive:
PAYLOADS = {
    'jis208': b'\x21\x21',      # ideographic space U+3000
    'jis212': b'\x21\x21',
    'ksc': b'\x21\x21',
    'gb': b'\x21\x21',
}
out = {}
for key in KEYS:
    res = []
    for d in DESIGS:
        bs = d + b'\x21\x21' + b'ab'
        try:
            t = bs.decode(key)
            res.append([d.hex(), t])
        except Exception as e:
            res.append([d.hex(), None, str(e)[:40]])
    out[key] = res
print(json.dumps(out))
'''
proc = subprocess.run([REF, "-S", "-c", src], capture_output=True, text=True, timeout=120)
if proc.returncode != 0:
    print(proc.stderr[:500], file=sys.stderr)
    sys.exit(1)
data = json.loads(proc.stdout.strip().splitlines()[-1])
for key, rows in data.items():
    print("==", key)
    for r in rows:
        if r[1] is not None:
            print("  OK", r[0], repr(r[1]))
        else:
            print("  --", r[0], r[2] if len(r) > 2 else "?")
