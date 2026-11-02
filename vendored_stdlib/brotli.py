"""antipythany deviation shim — brotli/brotlicffi (v0: identity storage
codec; the file:// gate never transfers brotli-encoded payloads)."""


def compress(data, mode=0, quality=11, lgwin=22, lgblock=0):
    return b"BRS1" + bytes(data)


def decompress(data):
    if data.startswith(b"BRS1"):
        return data[4:]
    raise ValueError("brotli shim: bad payload")


def decompressobj():
    class _O:
        def decompress(self, d):
            return decompress(d)

    return _O()
