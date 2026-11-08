"""Issue #13 end-to-end probe: the scrapy/w3lib response.text chain.

codecs.lookup('cp932') + encode/decode + w3lib-style resolve of an HTML
charset='gb2312' declaration -> decode the body. Run under the antipythony
binary; stderr markers delimit the phases; RC 0 = green.
"""
import codecs
import sys


def eprint(*a):
    print(*a, file=sys.stderr)


eprint("=== PHASE1: codecs.lookup ===")
for alias in ("cp932", "shift_jis", "euc_jp", "iso2022_jp", "big5", "big5hkscs",
              "cp949", "euc_kr", "johab", "gb2312", "gbk", "gb18030", "cp936",
              "cp950", "euc_cn", "hz", "iso2022_kr", "utf_7"):
    info = codecs.lookup(alias)
    eprint(f"  {alias:<12} -> {info.name}")

eprint("=== PHASE2: encode/decode ===")
b = "日本語テキスト".encode("cp932")
eprint("  cp932 encode:", b.hex())
eprint("  cp932 decode:", b.decode("cp932"))
gb = "中文内容".encode("gbk")
eprint("  gbk encode:", gb.hex(), "->", gb.decode("gbk"))

eprint("=== PHASE3: w3lib-style charset resolve ===")
# w3lib.encoding.resolve_encoding core: label -> canonical name -> decode.
html = (b"<html><head><meta charset='gb2312'></head>"
        b"<body>\xd6\xd0\xce\xc4\xc4\xda\xc8\xdd</body></html>")
declared = "gb2312"  # what html.parser/w3lib would extract from the meta tag
name = codecs.lookup(declared).name  # gb2312
body = html.split(b"<body>")[1].split(b"</body>")[0]
text = body.decode(name)
eprint(f"  charset='{declared}' -> codec '{name}'")
eprint("  body decoded:", text)
assert text == "中文内容", text

# the cp936 alias path (what requests/bs4 commonly pass)
name2 = codecs.lookup("cp936").name  # gbk
text2 = body.decode(name2)
eprint(f"  charset='cp936' -> codec '{name2}' -> {text2}")
assert text2 == "中文内容", text2

# iso-2022-jp email-style page
jp = ("<html><body>日本語</body></html>".encode("iso2022_jp")
      .decode("iso2022_jp"))
eprint("  iso2022_jp roundtrip:", jp)
assert jp == "<html><body>日本語</body></html>"

eprint("=== PROBE-GREEN ===")
