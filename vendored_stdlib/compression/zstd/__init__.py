"""antipythony deviation shim - compression.zstd (C-backed upstream)."""

def _unimplemented(*args, **kwargs):
    raise NotImplementedError("zstd compression not supported by the antipythony v0 shim")


class ZstdFile:
    def __init__(self, *args, **kwargs):
        _unimplemented()

    def read(self, *a, **k):
        _unimplemented()


def open(*args, **kwargs):
    _unimplemented()


def compress(*args, **kwargs):
    _unimplemented()


def decompress(*args, **kwargs):
    _unimplemented()
