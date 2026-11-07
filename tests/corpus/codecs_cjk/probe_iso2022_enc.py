#!/usr/bin/env python3
"""Verify SI/SO + GR-invocation for iso2022_kr (0x0F/0x0E shifts) and encode
shapes for iso2022_cn / iso2022_kr / hz / all jp family."""
import json
import subprocess
import sys

REF = "/opt/homebrew/bin/python3.14"

src = r'''
import json
res = {}
# iso2022_kr: SO (0x0E) enters KSC, SI (0x0F) leaves
bs = bytes.fromhex('1b2429430e4751315b0f6162')
res['kr_so_si'] = bs.decode('iso2022_kr')
# kr without SO: raw KSC bytes stay ASCII
bs2 = bytes.fromhex('1b2429434751315b')
try:
    res['kr_no_so'] = bs2.decode('iso2022_kr')
except Exception as e:
    res['kr_no_so'] = 'ERR:' + str(e)[:40]
# iso2022_jp encode shapes
for text in ['abc', 'あabc', 'abcあ', 'あ\r\nあ', 'あ\\', '~~あ']:
    res['jp_enc_' + repr(text)] = text.encode('iso2022_jp').hex()
# iso2022_kr encode
for text in ['abc', 'abc한', '한abc', '한\r\n한']:
    res['kr_enc_' + repr(text)] = text.encode('iso2022_kr').hex()
# iso2022_cn encode (gb2312 inside)
try:
    res['cn_enc'] = 'abc中'.encode('iso2022_cn').hex()
except Exception as e:
    res['cn_enc'] = 'ERR:' + str(e)[:50]
try:
    res['cn_cns_enc'] = 'abc中'.encode('iso2022_cn_cns').hex()
except Exception as e:
    res['cn_cns_enc'] = 'ERR:' + str(e)[:50]
# iso2022_jp_2 encode (ksc/gb chars)
try:
    res['jp2_enc_han'] = 'a한'.encode('iso2022_jp_2').hex()
except Exception as e:
    res['jp2_enc_han'] = 'ERR:' + str(e)[:50]
try:
    res['jp2_enc_zh'] = 'a中'.encode('iso2022_jp_2').hex()
except Exception as e:
    res['jp2_enc_zh'] = 'ERR:' + str(e)[:50]
try:
    res['jpext_enc_212'] = 'ü'.encode('iso2022_jp_ext').hex()
except Exception as e:
    res['jpext_enc_212'] = 'ERR:' + str(e)[:50]
# jis_3 / 2004 encodes with plane-2 char? (None known) just basic
res['jp3_enc'] = 'あ'.encode('iso2022_jp_3').hex()
res['jp2004_enc'] = 'あ'.encode('iso2022_jp_2004').hex()
res['jp3_enc_2132'] = '♥'.encode('iso2022_jp_3').hex() if True else None
# hz encode
for text in ['abc', 'abc中', '中abc', '~~', '中\r\n中']:
    try:
        res['hz_enc_' + repr(text)] = text.encode('hz').hex()
    except Exception as e:
        res['hz_enc_' + repr(text)] = 'ERR:' + str(e)[:50]
print(json.dumps(res, ensure_ascii=True))
'''
proc = subprocess.run([REF, "-S", "-c", src], capture_output=True, text=True, timeout=60)
print(proc.stdout)
if proc.returncode != 0:
    print(proc.stderr[:600], file=sys.stderr)
