"""lzma — minimal stdlib-compatible surface (antipythony vendored).

LZMA/xz compression. Used by feedexport postprocessing (test data).
Imports succeed; compression ops raise (same contract as the bz2 shim).
"""
__all__ = ["compress", "decompress", "open", "LZMAFile", "LZMACompressor",
           "LZMADecompressor", "FORMAT_XZ", "FORMAT_ALONE", "FORMAT_RAW"]


FORMAT_XZ = 1
FORMAT_ALONE = 2
FORMAT_RAW = 3


def compress(data, format=1, check=-1, preset=None, filters=None):
    raise NotImplementedError("lzma v0 shim: compression unimplemented")


def decompress(data, format=1, memlimit=None, filters=None):
    raise NotImplementedError("lzma v0 shim: compression unimplemented")


class LZMAFile:
    def __init__(self, filename, mode="r", format=None, check=-1, preset=None, filters=None):
        self.name = filename
        self.mode = mode

    def read(self, *a):
        raise NotImplementedError("lzma v0 shim")

    def write(self, data):
        raise NotImplementedError("lzma v0 shim")

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
        raise NotImplementedError("lzma v0 shim")

    def flush(self):
        raise NotImplementedError("lzma v0 shim")


class LZMADecompressor:
    def __init__(self, format=1, memlimit=None, filters=None):
        pass

    def decompress(self, data):
        raise NotImplementedError("lzma v0 shim")

    @property
    def eof(self):
        return False


def open(filename, mode="r", format=None, check=-1, preset=None, filters=None):
    return LZMAFile(filename, mode, format, check, preset, filters)
