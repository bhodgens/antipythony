"""antipythony deviation shim: minimal uuid surface (DEVIATIONS.toml).
The v1 gate (file:// scrapy) uses uuid4 for request fingerprints only."""
import os

RFC_4122 = 4


class UUID:
    __slots__ = ("int", "is_safe")

    def __init__(self, int=None, is_safe=0):
        if int is None:
            int = int.from_bytes(os.urandom(16), "big")
        self.int = int
        self.is_safe = is_safe

    def __repr__(self):
        return "UUID(%r)" % str(self)

    def __str__(self):
        h = "%032x" % self.int
        return "-".join(
            [h[0:8], h[8:12], h[12:16], h[16:20], h[20:32]]
        )

    def __eq__(self, other):
        return isinstance(other, UUID) and self.int == other.int

    def __hash__(self):
        return hash(self.int)

    @property
    def hex(self):
        return "%032x" % self.int

    @property
    def bytes(self):
        return self.int.to_bytes(16, "big")

    @property
    def version(self):
        return (self.int >> 76) & 0xF

    @property
    def variant(self):
        return RFC_4122


def uuid4():
    # RFC 4122 version 4: set version/variant bits on random bits.
    u = UUID(int=int.from_bytes(os.urandom(16), "big"))
    u.int = (u.int & 0xFFFFFFFFFFFF0FFF) | 0x4000
    u.int = (u.int & 0x3FFFFFFFFFFFFFFF) | (0x8 << 60)
    return u


def uuid1():
    return uuid4()


def uuid3(namespace, name):
    import hashlib
    h = hashlib.md5(bytes(namespace) + name.encode("utf-8")).digest()
    u = UUID(int=int.from_bytes(h, "big"))
    return u


def uuid5(namespace, name):
    import hashlib
    h = hashlib.sha1(bytes(namespace) + name.encode("utf-8")).digest()
    return UUID(int=int.from_bytes(h, "big"))


def _ns_uuid(i1, i2):
    # Compose the 128-bit namespace id from two 64-bit halves (the v1
    # bigint lexer chokes on long hex literals).
    return UUID(int=(i1 << 64) | i2)


_NSDNS1 = 0x6BA7B8109DAD11D1
_NSDNS2 = 0x80B400C04FD430C8
_NSURL1 = 0x6BA7B8119DAD11D1
_NSURL2 = 0x80B400C04FD430C8
_NSOID1 = 0x6BA7B8129DAD11D1
_NSOID2 = 0x80B400C04FD430C8
_NSX5001 = 0x6BA7B8149DAD11D1
_NSX5002 = 0x80B400C04FD430C8

NAMESPACE_DNS = _ns_uuid(_NSDNS1, _NSDNS2)
NAMESPACE_URL = _ns_uuid(_NSURL1, _NSURL2)
NAMESPACE_OID = _ns_uuid(_NSOID1, _NSOID2)
NAMESPACE_X500 = _ns_uuid(_NSX5001, _NSX5002)

RESERVED_NCS, RFC_4122_, RESERVED_MICROSOFT, RESERVED_FUTURE = 0, 2, 6, 7
