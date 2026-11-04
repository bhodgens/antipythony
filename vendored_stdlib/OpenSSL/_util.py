"""antipythony deviation shim — OpenSSL._util (v0)."""


class _Lib:
    def __init__(self):
        pass

    def __getattr__(self, item):
        def _call(*a, **k):
            return 0

        return _call


lib = _Lib()


def byte_string(s):
    return s if isinstance(s, bytes) else s.encode("utf-8")


def native(s):
    return s


def text_string(s):
    return s if isinstance(s, str) else s.decode("utf-8")
