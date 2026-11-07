#!/usr/bin/env python3
"""Probe which charset each ISO-2022 designation maps to, by payload char:
- JIS X 0208 0x2121 = U+3000
- GB2312 0x2121 = U+3000 too (ideographic space shared)
- KSC5601 0x2121 = U+3000 as well!
Use UNIQUE chars: 0x7426 jis208=?; instead probe a char unique per table:
- JIS X 0208 0x3642 = '和'  (0x3642)
- KSC5601 0x4771 = '걱'? use KSC 0x476E... We probe with euc_kr bytes of '한' = 0xc7d1 -> KSC5601 rows: (0xc7-0x80+? no; KSC5601 uses 0x21-0x7E trails.
- euc_kr '한' = 0xc7 0xd1 -> KSC5601 row/col: row = 0xc7-0x80? no: EUC-KR maps
  byte1 0xC7 -> KSC row 0x47, byte2 0xD1 -> col 0x51? (subtract 0x80).
  So KSC5601 0x47 0x51 = U+D55C (한).
- GB2312 '中' = 0xD6 0xD0 -> row 0x56 col 0x50: 5650 = U+4E2D.
- JIS X 0212 '亜' hmm. JIS X 0208 'あ' = 0x2422. JIS212 first char 0x2121 is also space.
  Use JIS X 0212 0x3173 = ? we know from earlier probe: 1b242844 4c45 -> '瑨' (U+7968?)
  earlier: 4C45 -> '瑨' = U+7A68. so JIS212 uses raw 2 bytes -> we can identify by output.
"""
import json
import subprocess
import sys

REF = "/opt/homebrew/bin/python3.14"

# payload bytes chosen so the DECODED char identifies the charset:
# - 2121: ideographic space in all CJK tables -> ambiguous, skip
# - jis208 row 0x24 col 0x22 = 'あ' (U+3042) ONLY in JIS X 0208
# - jis208 row 0x36 col 0x42 = '和'? verify under euc_jp first.
# - ksc row 0x47 col 0x51 = '한' (U+D55C) ONLY in KSC5601
# - gb row 0x56 col 0x50 = '中' (U+4E2D) ONLY in GB2312
# - jis212 row 0x4c col 0x45 = U+7A68 ONLY in JIS X 0212
src = r'''
import json
KEYS = ['iso2022_jp', 'iso2022_jp_1', 'iso2022_jp_2', 'iso2022_jp_3',
        'iso2022_jp_ext', 'iso2022_jp_2004', 'iso2022_kr']
DESIGS = [
    b'\x1b$B', b'\x1b$@', b'\x1b$A', b'\x1b$)C', b'\x1b$)A', b'\x1b$)G',
    b'\x1b$(B', b'\x1b$(C', b'\x1b$(D', b'\x1b$(O', b'\x1b$(P', b'\x1b$(Q',
]
PAYLOADS = [
    ('jis208', b'\x24\x22'),
    ('ksc', b'\x47\x51'),
    ('gb', b'\x56\x50'),
    ('jis212', b'\x4c\x45'),
    ('jis213p1', b'\x21\x6c'),
]
out = {}
for key in KEYS:
    res = []
    for d in DESIGS:
        for pname, pay in PAYLOADS:
            bs = d + pay + b'ab'
            try:
                t = bs.decode(key)
                res.append([d.hex(), pname, t])
            except Exception:
                pass
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
    for d, pname, t in rows:
        print(f"  {d} {pname} -> {t!r}")
