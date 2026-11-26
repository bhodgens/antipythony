"""bz2 — stdlib-compatible surface (antipythony vendored).

pp2-bz2: the native module registry OVERRIDES this file with a real
one-shot surface (bz2.compress/decompress via the pure-Rust bzip2 crate;
compress output byte-identical to CPython, error texts CPython-exact).
This vendored file remains as the fallback face for the excluded streaming
objects (BZ2File/BZ2Compressor/BZ2Decompressor raise lazily, same contract
as the zlib streaming-objects shim).
"""
__all__ = ["compress", "decompress", "open", "BZ2File", "BZ2Decompressor",
           "BZ2Compressor"]


def compress(data, compresslevel=9):
    raise NotImplementedError("bz2 streaming/file objects are not supported by the antipythony v1 runtime; one-shot compress/decompress are native")

class BZ2File:
    def __init__(self, filename, mode="r", buffering=None, compresslevel=9):
        self.name = filename
        self.mode = mode

    def read(self, *a):
        raise NotImplementedError("bz2 file objects are not supported by the antipythony v1 runtime")

    def write(self, data):
        raise NotImplementedError("bz2 file objects are not supported by the antipythony v1 runtime")

    def close(self):
        return None

    def __enter__(self):
        return self

    def __exit__(self, *a):
        self.close()
        return False


class BZ2Compressor:
    def __init__(self, compresslevel=9):
        pass

    def compress(self, data):
        raise NotImplementedError("bz2 streaming objects are not supported by the antipythony v1 runtime")

    def flush(self):
        raise NotImplementedError("bz2 streaming objects are not supported by the antipythony v1 runtime")


class BZ2Decompressor:
    def __init__(self):
        pass

    def decompress(self, data):
        raise NotImplementedError("bz2 streaming objects are not supported by the antipythony v1 runtime")

    @property
    def eof(self):
        return False


def open(filename, mode="r", compresslevel=9):
    return BZ2File(filename, mode, None, compresslevel)
