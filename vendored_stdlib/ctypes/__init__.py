"""antipythony deviation shim — ctypes (v0: the file:// gate's
scrapy.utils.display only probes libc for terminal width; everything else
is inert)."""


class _CDLL:
    def __init__(self, name=None, *a, **k):
        self._name = name

    def __getattr__(self, item):
        def _call(*a, **k):
            return 0

        return _call


class _LibraryLoader:
    def LoadLibrary(self, name):
        return _CDLL(name)

    def find_library(self, name):
        return None


cdll = _LibraryLoader()
CDLL = _CDLL
windll = _LibraryLoader()
oledll = _LibraryLoader()
pydll = _LibraryLoader()


class _CType:
    _name = None

    def __init__(self, name="c_int", *a):
        self.__name__ = name
        self._name = name

    def __call__(self, *a):
        return self


def _make_type(name="c_int"):
    return _CType(name)


c_int = _make_type("c_int")
c_uint = _make_type("c_uint")
c_long = _make_type("c_long")
c_ulong = _make_type("c_ulong")
c_char_p = _make_type("c_char_p")
c_wchar_p = _make_type("c_wchar_p")
c_void_p = _make_type("c_void_p")
c_bool = _make_type("c_bool")
c_float = _make_type("c_float")
c_double = _make_type("c_double")
c_byte = _make_type("c_byte")
c_ubyte = _make_type("c_ubyte")
c_short = _make_type("c_short")
c_ushort = _make_type("c_ushort")


class Structure:
    def __init__(self, *a, **k):
        pass


class Union:
    def __init__(self, *a, **k):
        pass


class POINTER:
    def __init__(self, t=None):
        self._t = t

    def __call__(self, *a):
        return self

    def contents(self):
        return None


pointer = POINTER


class _Func:
    def __init__(self, *a, **k):
        pass

    def __call__(self, *a, **k):
        return 0


CFUNCTYPE = _Func
PYFUNCTYPE = _Func
WINFUNCTYPE = _Func


def get_errno():
    return 0


def set_errno(v):
    return None


def util_find_library(name):
    return None


class util:
    find_library = staticmethod(util_find_library)


def sizeof(t):
    return 8


def byref(t):
    return t


def create_string_buffer(init, size=None):
    if size is None:
        return bytearray(bytes(init, "utf-8") if isinstance(init, str) else bytes(init))
    return bytearray(size)


def create_unicode_buffer(init, size=None):
    if size is None:
        s = init if isinstance(init, str) else str(init)
        buf = []
        buf.extend(s)
        buf.append("\x00")
        return buf
    return ["" for _ in range(size)]


def string_at(ptr, size=-1):
    return b""


def memmove(dst, src, count):
    return None


def cast(t, obj):
    return obj
