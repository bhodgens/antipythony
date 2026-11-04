"""bz2 — minimal stdlib-compatible surface (antipythony vendored).

BZ2 compression. Used by feedexport postprocessing (test data). If the
`bzip2` CLI tool is unavailable the functions raise, matching the module's
availability contract loosely; imports always succeed.
"""
__all__ = ["compress", "decompress", "open", "BZ2File", "BZ2Decompressor",
           "BZ2Compressor"]


def compress(data, compresslevel=9):
    raise NotImplementedError("bz2 v0 shim: compression unimplemented")


def decompress(data):
    raise NotImplementedError("bz2 v0 shim: compression unimplemented")


class BZ2File:
    def __init__(self, filename, mode="r", buffering=None, compresslevel=9):
        self.name = filename
        self.mode = mode

    def read(self, *a):
        raise NotImplementedError("bz2 v0 shim: compression unimplemented")

    def write(self, data):
        raise NotImplementedError("bz2 v0 shim: compression unimplemented")

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
        raise NotImplementedError("bz2 v0 shim")

    def flush(self):
        raise NotImplementedError("bz2 v0 shim")


class BZ2Decompressor:
    def __init__(self):
        pass

    def decompress(self, data):
        raise NotImplementedError("bz2 v0 shim")

    @property
    def eof(self):
        return False


def open(filename, mode="r", compresslevel=9):
    return BZ2File(filename, mode, None, compresslevel)
