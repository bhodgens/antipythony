"""antipythany deviation shim — gzip (v0: zlib-shim storage framing)."""

import struct
import zlib

_MAGIC = b"GZS1"


def _frame(data):
    return _MAGIC + struct.pack("<I", len(data)) + bytes(data)


def _unframe(data):
    if not data.startswith(_MAGIC):
        raise ValueError("gzip shim: bad payload")
    (ln,) = struct.unpack("<I", data[4:8])
    return data[8:8 + ln]


class GzipFile:
    def __init__(self, filename=None, mode=None, compresslevel=9,
                 fileobj=None, mtime=None):
        self.fileobj = fileobj
        self.mode = mode
        self.name = filename
        self._buf = bytearray()

    def write(self, data):
        self._buf += bytes(data)

    def read(self, size=-1):
        out = _unframe(bytes(self.fileobj.read())) if self.fileobj else b""
        return out if size < 0 else out[:size]

    def close(self):
        if self.fileobj is not None and self._buf:
            self.fileobj.write(_frame(bytes(self._buf)))
            self._buf = bytearray()

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()


def open(filename, mode="rb", compresslevel=9, encode=None, errors=None,
         newline=None):
    return GzipFile(filename=filename, mode=mode)


def compress(data, compresslevel=9, *, mtime=None):
    return _frame(data)


def decompress(data, bufsize=0):
    return _unframe(data)
