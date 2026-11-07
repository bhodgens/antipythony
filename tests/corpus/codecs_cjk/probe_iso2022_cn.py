#!/usr/bin/env python3
"""Probe iso2022_cn / iso2022_cn_cns / gb2312-euc mapping details and cp932
NEC/IBM extension rows."""
import json
import subprocess
import sys

REF = "/opt/homebrew/bin/python3.14"

src = r'''
import json
res = {}
# iso2022_cn decode shapes
for hx in ('1b2429414650', '1b2429474650', '0e46500f61'):
    bs = bytes.fromhex(hx)
    try:
        res['cn_dec_' + hx] = bs.decode('iso2022_cn')
    except Exception as e:
        res['cn_dec_' + hx] = 'ERR:' + str(e)[:50]
# cp932 NEC/IBM rows: NEC row 0x87, IBM rows 0xFA-0xFC
for hx in ('8740', 'fa40', 'fbfc', 'eeef'):
    bs = bytes.fromhex(hx)
    try:
        res['cp932_' + hx] = bs.decode('cp932')
    except Exception as e:
        res['cp932_' + hx] = 'ERR:' + str(e)[:40]
# shift_jis (not cp932): NEC/IBM rows absent
for hx in ('8740', 'fa40', 'fbfc', '8771'):
    bs = bytes.fromhex(hx)
    try:
        res['sjis_' + hx] = bs.decode('shift_jis')
    except Exception as e:
        res['sjis_' + hx] = 'ERR:' + str(e)[:40]
# cp932 encode of NEC-special chars (U+2266? no: NEC row maps 8741..=U+2252? probe)
for text in ['\u2252', '\u2261', '\u00a1', '\uffe2']:
    try:
        res['cp932_enc_' + hex(ord(text))] = text.encode('cp932').hex()
    except Exception as e:
        res['cp932_enc_' + hex(ord(text))] = 'ERR:' + str(e)[:40]
print(json.dumps(res, ensure_ascii=True))
'''
proc = subprocess.run([REF, "-S", "-c", src], capture_output=True, text=True, timeout=60)
print(proc.stdout)
if proc.returncode != 0:
    print(proc.stderr[:600], file=sys.stderr)
