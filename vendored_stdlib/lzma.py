"""lzma — stdlib-compatible surface (antipythony vendored).

pp2-bz2: the native module registry OVERRIDES this file with a real
one-shot surface (lzma.compress/decompress over FORMAT_XZ/ALONE/RAW via
the pure-Rust lzma-rs crate; CPython-exact error taxonomy; compress
output decompress-compatible but not byte-identical to liblzma — the
lzma-rs range-encoder makes different choices). This vendored file
remains as the fallback face for the excluded streaming objects
(LZMAFile/LZMACompressor/LZMADecompressor raise lazily).
"""
__all__ = ["compress", "decompress", "open", "LZMAFile", "LZMACompressor",
           "LZMADecompressor", "FORMAT_XZ", "FORMAT_ALONE", "FORMAT_RAW"]


FORMAT_AUTO = 0
FORMAT_XZ = 1
FORMAT_ALONE = 2
FORMAT_RAW = 3


def compress(data, format=1, check=-1, preset=None, filters=None):
    raise NotImplementedError("lzma streaming/file objects are not supported by the antipythony v1 runtime; one-shot compress/decompress are native")


class LZMAFile:
    def __init__(self, filename, mode="r", format=None, check=-1, preset=None, filters=None):
        self.name = filename
        self.mode = mode

    def read(self, *a):
        raise NotImplementedError("lzma file objects are not supported by the antipythony v1 runtime")

    def write(self, data):
        raise NotImplementedError("lzma file objects are not supported by the antipythony v1 runtime")

    def close(self):
        return None

    def __enter__(self):
        return self

    def __exit__(self, *a):
        self.close()
        return False


class LZMACompressor:
    def __init__(self, format=1, check=-1, preset=None, filters=None):
        pass

    def compress(self, data):
        raise NotImplementedError("lzma streaming objects are not supported by the antipythony v1 runtime")

    def flush(self):
        raise NotImplementedError("lzma streaming objects are not supported by the antipythony v1 runtime")


class LZMADecompressor:
    def __init__(self, format=1, memlimit=None, filters=None):
        pass

    def decompress(self, data):
        raise NotImplementedError("lzma streaming objects are not supported by the antipythony v1 runtime")

    @property
    def eof(self):
        return False


def open(filename, mode="r", format=None, check=-1, preset=None, filters=None):
    return LZMAFile(filename, mode, format, check, preset, filters)
