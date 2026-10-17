# Cites: S-05-4 (rows keyword, codecs+encodings bootstrap subset)
# Slice: stdlib-v1 (T-11, gates 02.08). Reference bytes pinned by
# /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the C-01 pin 3.14.0,
# per the T-05/T-06 provenance-gap precedent).
import codecs
import encodings
import keyword
names_k = ["iskeyword", "issoftkeyword", "kwlist", "softkwlist"]
print(all(hasattr(keyword, n) for n in names_k))
print(keyword.iskeyword("lambda"), keyword.iskeyword("_"),
      keyword.issoftkeyword("_"), keyword.issoftkeyword("case"),
      len(keyword.kwlist))
print(all(isinstance(k, str) for k in keyword.kwlist),
      sorted(keyword.kwlist)[:4])
names_c = ["lookup", "register", "encode", "decode", "open", "register_error",
           "Codec", "BOM", "BOM_UTF8", "BOM_UTF16", "BOM_UTF16_BE",
           "BOM_UTF16_LE", "BOM_UTF32", "IncrementalEncoder",
           "IncrementalDecoder", "BufferedIncrementalEncoder",
           "BufferedIncrementalDecoder", "StreamReader", "StreamWriter"]
print(len(names_c), all(hasattr(codecs, n) for n in names_c))
print(codecs.lookup("UTF-8").name, codecs.lookup("utf8").name,
      codecs.lookup("latin-1").name, codecs.lookup("ascii").name,
      codecs.lookup("utf-16").name)
print(codecs.encode("é", "utf-8"), codecs.decode(b"\xc3\xa9", "utf-8"))
print(codecs.BOM_UTF8, codecs.BOM_UTF16_BE, codecs.BOM_UTF16_LE,
      codecs.BOM_UTF32)
print("é".encode("latin-1").hex(),
      codecs.lookup("latin-1").encode("é")[0].hex())
print(len(codecs.lookup("utf-32").encode("a")[0]))
print(codecs.BOM_BE is not None, codecs.BOM_LE is not None)
ie = codecs.getincrementaldecoder("utf-8")()
print(repr(ie.decode(b"\xc3") + ie.decode(b"\xa9")))
ee = codecs.getincrementalencoder("utf-8")()
print(repr(ee.decode if False else ee.encode("é") + ee.encode("", True)))
print(encodings.aliases.aliases.get("latin1"),
      encodings.aliases.aliases.get("U8", "?"),
      encodings.aliases.aliases.get("utf-8"))
try:
    codecs.lookup("quixotic-codec-t11")
except LookupError as e:
    print("codec LookupError:", str(e)[:44])
print(codecs.lookup("utf-8").incrementalencoder is not None)
