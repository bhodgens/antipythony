"""Differential probe: antipythony vs CPython 3.14 ground truth for the
native multibyte CJK codecs. Compares codecs.lookup canonical names, encode
bytes, decode results, error handlers, partial-byte sequences, and the
w3lib-style gb2312 HTML-charset resolution. Exits non-zero on any mismatch.
"""
import codecs
import json
import sys

GT = json.load(open("tests/corpus/codecs_cjk/ground_truth_py314.json"))

fails = []
checks = 0


def check(label, got, want):
    global checks
    checks += 1
    if got != want:
        fails.append(f"{label}: got {got!r} want {want!r}")


# 1. lookup canonical names for every alias
ALIASES = {
    "cp932": "cp932", "shift_jis": "shift_jis", "euc_jp": "euc_jp",
    "iso2022_jp": "iso2022_jp", "iso2022_jp_1": "iso2022_jp_1",
    "iso2022_jp_2": "iso2022_jp_2", "iso2022_jp_3": "iso2022_jp_3",
    "iso2022_jp_ext": "iso2022_jp_ext", "iso2022_jp_2004": "iso2022_jp_2004",
    "iso2022_kr": "iso2022_kr", "big5": "big5", "big5hkscs": "big5hkscs",
    "cp949": "cp949", "euc_kr": "euc_kr", "johab": "johab",
    "gb2312": "gb2312", "gbk": "gbk", "gb18030": "gb18030", "cp936": "gbk",
    "cp950": "cp950", "euc_cn": "gb2312", "hz": "hz",
    "shift_jisx0213": "shift_jisx0213", "shift_jis_2004": "shift_jis_2004",
    "euc_jisx0213": "euc_jisx0213", "euc_jis_2004": "euc_jis_2004",
}
for alias, canon in ALIASES.items():
    try:
        info = codecs.lookup(alias)
        check(f"lookup({alias})", info.name, canon)
    except Exception as e:
        fails.append(f"lookup({alias}): {type(e).__name__}: {e}")

# 2. encode ground truth (from the GT file) + decode round-trip
import codecs as _c
SAMPLES = ["hello", "\u4e2d\u6587", "\u65e5\u672c\u8a9e", "\ud55c\uad6d\uc5b4",
           "\uff21\uff22", "\u00e9\u20ac", "\u9ad8\u5fb7\u7eb2", "\uac00\uac01",
           "\u203b", "\u30e2\u30fc\u30b7\u30e7\u30f3", "\u4e00\u4e01",
           "\u30fb", "\u2116"]
for key, entry in GT.items():
    for samp_repr, hexs in entry.get("enc", {}).items():
        samp = eval(samp_repr)  # repr from the generator (safe subset)
        want = bytes.fromhex(hexs) if hexs else None
        try:
            got = samp.encode(key)
        except UnicodeEncodeError:
            got = None
        check(f"enc[{key}]{samp_repr}", got, want)
    for hexs, dec in entry.get("dec", {}).items():
        b = bytes.fromhex(hexs)
        want = dec[0] if dec else None
        try:
            got = b.decode(key)
        except UnicodeDecodeError:
            got = None
        check(f"dec[{key}] {hexs}", got, want)

# 3. error handlers on cp932 (the w3lib-relevant codec)
check("enc-replace", "café".encode("cp932", "replace"), b"caf?")
check("enc-ignore", "café".encode("cp932", "ignore"), b"caf")
check("enc-bslash", "café".encode("cp932", "backslashreplace"), b"caf\\xe9")
check("enc-xmlref", "café".encode("cp932", "xmlcharrefreplace"), b"caf&#233;")
try:
    "café".encode("cp932")
    fails.append("enc-strict: no raise")
except UnicodeEncodeError as e:
    check("enc-strict-msg", str(e).splitlines()[0],
          "'cp932' codec can't encode character '\\xe9' in position 3: illegal multibyte sequence")
try:
    b"\x81\x00".decode("cp932")
    fails.append("dec-strict: no raise")
except UnicodeDecodeError as e:
    check("dec-strict-msg", str(e).splitlines()[0],
          "'cp932' codec can't decode byte 0x81 in position 0: illegal multibyte sequence")
check("dec-replace", b"\x81\x00".decode("cp932", "replace"), "\ufffd\x00")
check("dec-ignore", b"\x81\x00".decode("cp932", "ignore"), "\x00")

# 4. partial-byte sequences (incremental decoder)
inc = codecs.getincrementaldecoder("cp932")()
p1 = inc.decode(b"\x93")
p2 = inc.decode(b"\xfa", True)
check("incr-cp932", p1 + p2, "日")
inc2 = codecs.getincrementaldecoder("gbk")()
q1 = inc2.decode(b"\xd6")
q2 = inc2.decode(b"\xd0", True)
check("incr-gbk", q1 + q2, "中")

# 5. hz escape framing (orchestrator-verified shape)
check("hz-enc", "中".encode("hz"), bytes.fromhex("7e7b56507e7d"))
check("hz-roundtrip", bytes.fromhex("7e7b56507e7d").decode("hz"), "中")
check("hz-ascii", "ab".encode("hz"), b"ab")
check("hz-tilde", "~".encode("hz"), b"~~")

# 6. iso2022_kr SO/SI framing (orchestrator-verified shape)
check("kr-enc", "a한b".encode("iso2022_kr"), bytes.fromhex("611b2429430e47510f62"))
check("kr-dec", bytes.fromhex("611b2429430e47510f62").decode("iso2022_kr"), "a한b")
check("jp-enc", "abc雨".encode("iso2022_jp"), bytes.fromhex("6162631b2442312b1b2842"))
check("jp-dec", bytes.fromhex("6162631b2442312b1b2842").decode("iso2022_jp"), "abc雨")

# 7. gb18030 four-byte (orchestrator-owned engine)
check("gb4-enc", "\U0001f600".encode("gb18030"), bytes.fromhex("9439fc36"))
check("gb4-dec", bytes.fromhex("9439fc36").decode("gb18030"), "\U0001f600")
check("gb4-u80", "\x80".encode("gb18030"), bytes.fromhex("81308130"))
check("gb4-dec80", bytes.fromhex("81308130").decode("gb18030"), "\x80")

# 8. euc_jp / johab regenerated tables spot check (orchestrator-verified)
check("eucjp-enc", "中".encode("euc_jp"), bytes.fromhex("c3e6"))
check("eucjp-dec", bytes.fromhex("c3e6").decode("euc_jp"), "中")
check("johab-enc", "中".encode("johab"), bytes.fromhex("f3e9"))

# 9. w3lib-style resolve: html charset='gb2312' -> codec name -> decode
html = b"<html><meta charset='gb2312'><body>\xd6\xd0\xce\xc4</body></html>"
label = "gb2312"
name = codecs.lookup(label).name  # w3lib resolve_encoding core
body = html.split(b"<body>")[1].split(b"</body>")[0]
check("w3lib-resolve", name, "gb2312")
check("w3lib-decode", body.decode(name), "中文")
# cp936 alias path too
check("w3lib-cp936", codecs.lookup("cp936").name, "gbk")
check("w3lib-cp936-dec", body.decode("cp936"), "中文")

# 10. utf_7 (pure-Python wrapper path over the native _codecs.utf_7_* prims)
# CPython ground truth: '+' -> '+-'; CJK -> modified-base64 runs.
check("utf7-enc", "a+b/c".encode("utf_7"), b"a+-b/c")
check("utf7-enc-cjk", "中a".encode("utf_7"), b"+Ti0-a")
check("utf7-roundtrip", b"+Ti0-a".decode("utf_7"), "中a")
check("utf7-lookup", codecs.lookup("utf_7").name, "utf-7")
check("utf7-alias-u7", codecs.lookup("u7").name, "utf-7")

print(f"checks: {checks}, fails: {len(fails)}")
for f in fails:
    print("FAIL", f)
sys.exit(1 if fails else 0)
