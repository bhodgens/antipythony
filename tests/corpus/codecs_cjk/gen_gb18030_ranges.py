#!/usr/bin/env python3
"""Determine gb18030 linear 4-byte ranges empirically from CPython."""
import subprocess
import json
import sys

REF = "/opt/homebrew/bin/python3.14"

src = r'''
import json
key = 'gb18030'
def enc4(cp):
    ch = chr(cp)
    b = ch.encode(key)
    if len(b) == 4:
        return b.hex()
    return None
# probe the linear ranges: find boundaries by bisection over a range
def find_range(start, end):
    first = enc4(start)
    last = None
    lastcp = None
    for cp in range(start, end + 1):
        h = enc4(cp)
        if h is not None:
            last = h
            lastcp = cp
        else:
            if last is not None and cp - 1 >= start:
                break
    return first, lastcp, last
# contiguous encodable-in-4-byte ranges start after BMP covered by 2-byte tables.
# probe known boundaries (from the GB18030-2005 spec as implemented in CPython)
ranges = []
# We probe a coarse map: for every 64-cp block in BMP+ supplementary, does it encode 4-byte?
blocks = []
cur = None
for cp in range(0x80, 0x110000):
    ch = chr(cp)
    try:
        b = ch.encode(key)
        n = len(b)
    except Exception:
        n = None
    if cur is None or cur[1] != n:
        if cur is not None:
            blocks.append([cur[0], cur[1], cp - 1])
        cur = [cp, n]
    else:
        cur[1] = n
if cur is not None:
    blocks.append([cur[0], cur[1], 0x10ffff])
print(json.dumps(blocks))
'''
proc = subprocess.run([REF, "-S", "-c", src], capture_output=True, text=True, timeout=600)
if proc.returncode != 0:
    print(proc.stderr[:500], file=sys.stderr)
    sys.exit(1)
blocks = json.loads(proc.stdout.strip().splitlines()[-1])
# blocks: list of [start_cp, len] with an end marker appended; reconstruct.
out = []
for i, b in enumerate(blocks):
    start, n = b[0], b[1]
    end = blocks[i + 1][0] - 1 if i + 1 < len(blocks) else 0x10ffff
    out.append([hex(start), hex(end), n])
for row in out:
    if row[2] == 4:
        print(row)
