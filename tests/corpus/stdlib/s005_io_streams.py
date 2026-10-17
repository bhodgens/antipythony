# Cites: S-05-4 (spec 05 §5.3.1 row io), S-06-19
# Slice: stdlib-v1 (T-11, gates 02.08). Reference bytes pinned by
# /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the C-01 pin 3.14.0,
# per the T-05/T-06 provenance-gap precedent).
import io
import os
names = ["IOBase", "RawIOBase", "BufferedIOBase", "TextIOBase",
         "BufferedReader", "BufferedWriter", "BufferedRWPair",
         "BufferedRandom", "TextIOWrapper", "BytesIO", "StringIO", "FileIO",
         "UnsupportedOperation", "BlockingIOError", "DEFAULT_BUFFER_SIZE",
         "SEEK_SET", "SEEK_CUR", "SEEK_END", "open",
         "IncrementalNewlineDecoder", "text_encoding"]
print(all(hasattr(io, n) for n in names))
print(io.DEFAULT_BUFFER_SIZE, io.SEEK_SET, io.SEEK_CUR, io.SEEK_END)
print(io.text_encoding(None), io.text_encoding("utf-8"))
b = io.BytesIO(b"hello world")
print(b.read(5), b.tell(), b.read(), b.tell(), b.seek(0, io.SEEK_END), b.tell())
s = io.StringIO("line1\nline2\n")
print(s.readline().strip(), s.read().strip())
s.seek(0)
print([ln.strip() for ln in s])
t = io.StringIO()
print(t.write("x"), t.getvalue())
print(issubclass(io.BytesIO, io.RawIOBase),
      issubclass(io.BufferedReader, io.BufferedIOBase),
      issubclass(io.TextIOWrapper, io.TextIOBase))
fw = io.open(os.devnull, "w")
try:
    fw.read()
except io.UnsupportedOperation as e:
    print("UnsupportedOperation:", e)
finally:
    fw.close()
bi = io.BytesIO("héllo".encode("utf-8"))
tw = io.TextIOWrapper(bi, encoding="utf-8")
print(tw.read(), tw.encoding)
print(io.IncrementalNewlineDecoder(None, True).decode("a\r\nb"))
br = io.BufferedReader(io.BytesIO(b"0123456789"))
print(br.read(3), br.read1(2) if hasattr(br, "read1") else "noread1", br.peek(1)[:1])
import tempfile
tf = io.FileIO(tempfile.mkstemp(suffix=".t11")[1], "w")
tf.write(b"xy"); tf.close()
print(type(io.open).__name__ != "", callable(io.FileIO))
