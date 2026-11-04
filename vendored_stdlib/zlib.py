"""antipythony deviation shim - zlib (upstream is a C module; docs/lxml-shim.md).
Import-level surface only for the T1 file:// gate (no network compression fires).
Real compression raises NotImplementedError lazily."""

MAX_WBITS = 15
DEFLATED = 8
BEST_SPEED = 1
BEST_COMPRESSION = 9
Z_DEFAULT_COMPRESSION = -1
Z_OK = 0
Z_STREAM_END = 1
Z_NO_FLUSH = 0
Z_SYNC_FLUSH = 2
Z_FINISH = 4
Z_FULL_FLUSH = 3
Z_BEST_SPEED = 1
Z_BEST_COMPRESSION = 9
Z_DEFAULT_STRATEGY = 0
Z_FILTERED = 1
Z_HUFFMAN_ONLY = 2
Z_RLE = 3
Z_FIXED = 4
ZSTORED = 0
Z_DEFLATED = 8


class error(Exception):
    pass


def _unimplemented(*args, **kwargs):
    raise NotImplementedError("zlib compression not supported by the antipythony v0 shim")


compress = _unimplemented
decompress = _unimplemented
decompressobj = _unimplemented
compressobj = _unimplemented
crc32 = lambda data, value=0: 0  # placeholder: callers only compare/track
adler32 = lambda data, value=0: 0
