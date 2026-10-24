# antipythany deviation shim (DEVIATIONS.toml): the real socket module wraps
# the _socket C extension (docs/compat-contract.md C-07). The T1 gate crawls
# file:// URLs only, so network calls never fire; this shim provides the
# imported names (constants, error types, trivial helpers) that http.client
# and urllib.request touch at import time.
class error(OSError):
    pass


class herror(error):
    pass


class gaierror(error):
    pass


class timeout(error):
    pass


AF_INET = 2
AF_INET6 = 30
AF_UNIX = 1
AF_UNSPEC = 0
AF_APPLETALK = 16
AF_DECnet = 12
AF_INET6 = 30
AF_IPX = 23
AF_SNA = 22
AI_PASSIVE = 1
AI_CANONNAME = 2
AI_NUMERICHOST = 4
AI_NUMERICSERV = 8
AI_V4MAPPED = 8
AI_ALL = 16
AI_ADDRCONFIG = 32
IPPROTO_IP = 0
IPPROTO_ICMP = 1
IPPROTO_TCP = 6
IPPROTO_UDP = 17
IPPROTO_IPV6 = 41
IPPORT_RESERVED = 1024
MSG_OOB = 1
MSG_PEEK = 2
MSG_DONTROUTE = 4
MSG_WAITALL = 8
MSG_EOR = 16
MSG_TRUNC = 0x10
MSG_CTRUNC = 0x20
SO_DEBUG = 1
SO_ACCEPTCONN = 2
SO_DONTROUTE = 16
SO_BROADCAST = 32
SO_USELOOPBACK = 64
SO_LINGER = 128
SO_OOBINLINE = 256
SO_SNDBUF = 0x1001
SO_RCVBUF = 0x1002
SO_SNDLOWAT = 0x1003
SO_RCVLOWAT = 0x1004
SO_SNDTIMEO = 0x1005
SO_RCVTIMEO = 0x1006
SO_ERROR = 0x1007
SO_TYPE = 0x1008
SOL_IP = 0
SOL_TCP = 6
SOL_UDP = 17
TCP_NODELAY = 1
TCP_MAXSEG = 2
TCP_CORK = 3
TCP_KEEPIDLE = 0x10
TCP_KEEPINTVL = 0x20
TCP_KEEPCNT = 0x40
TCP_SYNCNT = 0x70
TCP_LINGER2 = 0x80
TCP_DEFER_ACCEPT = 0x100
TCP_WINDOW_CLAMP = 0x200
TCP_INFO = 0x200
TCP_QUICKACK = 0x400
SHUT_WR = 1
SHUT_RDWR = 2
INADDR_ANY = 0
INADDR_BROADCAST = 0xFFFFFFFF
INADDR_LOOPBACK = 0x7F000001
INADDR_NONE = 0xFFFFFFFF
SOCK_STREAM = 1
SOCK_DGRAM = 2
SOL_SOCKET = 0xFFFF
SO_REUSEADDR = 4
SHUT_RD = 0
SHUT_WR = 1
SHUT_RDWR = 2
IPPROTO_IP = 0
IPPROTO_TCP = 6
TCP_NODELAY = 1
_HAS_IPv6 = True


def gethostname():
    return "localhost"


def gethostbyname(name):
    return "127.0.0.1"


# T1-d (twisted.internet._resolver): address-family / socket-kind constants.
AF_UNSPEC = 0
AF_INET6 = 30
AF_UNIX = 1
SOCK_DGRAM = 2
AddressFamily = int
SocketKind = int
gaierror = OSError

AI_NUMERICHOST = 1
AI_PASSIVE = 2
AI_CANONNAME = 4
AI_NUMERICSERV = 8
AI_V4MAPPED = 16
AI_ALL = 32
AI_ADDRCONFIG = 64


def getaddrinfo(*args, **kwargs):
    # Inert: the file:// gate makes no real DNS/connections.
    return [(AF_INET, SOCK_STREAM, 6, "", ("127.0.0.1", 0))]


def create_connection(*args, **kwargs):
    raise NotImplementedError("network sockets are not supported in this runtime")


class _SocketShim:
    """T1-d: inert socket object — the file:// gate never sends traffic;
    twisted's imports (tcp.py Socket.bind etc.) need the method surface."""

    def __init__(self, family=AF_INET, type=SOCK_STREAM, proto=0, fileno=None):
        self.family = family
        self.type = type
        self.proto = proto
        self.fileno_ = fileno if fileno is not None else -1
        self.bound = False
        self.closed = False

    def bind(self, address):
        self.bound = True

    def close(self):
        self.closed = True

    def setsockopt(self, level, optname, value):
        return None

    def getsockopt(self, level, optname, default=None):
        return default if default is not None else 0

    def setblocking(self, flag):
        return None

    def settimeout(self, value):
        return None

    def gettimeout(self):
        return None

    def listen(self, backlog=128):
        return None

    def accept(self):
        # T1-d: non-blocking inert socket — report EAGAIN like CPython so
        # reactors treat "no connections" as normal.
        raise OSError(35, "Resource temporarily unavailable")

    def connect(self, address):
        # T1-d: non-blocking inert connect — EINPROGRESS then immediate
        # ECONNREFUSED on check; simplest: ECONNREFUSED.
        raise OSError(61, "Connection refused")

    def connect_ex(self, address):
        return errno.ECONNREFUSED

    def send(self, data, flags=0):
        raise NotImplementedError("network sockets are not supported in this runtime")

    def sendall(self, data, flags=0):
        raise NotImplementedError("network sockets are not supported in this runtime")

    def recv(self, bufsize, flags=0):
        return b""

    def fileno(self):
        return self.fileno_

    def shutdown(self, how):
        self.closed = True

    def getsockname(self):
        return ("127.0.0.1", 0)

    def getpeername(self):
        return ("127.0.0.1", 0)

    def detach(self):
        return self.fileno_


def socket(family=AF_INET, type=SOCK_STREAM, proto=0, fileno=None):
    return _SocketShim(family, type, proto, fileno)


# T1-d (twisted.internet.base / selectors): address-family helpers the
# file://-only gate never exercises. Pure shims: inet_pton/inet_ntop for
# the common IPv4 dotted-quad form, inet_ntoa/inet_aton likewise.
def _check_ipv4(packed):
    if len(packed) != 4:
        raise OSError("packed IP wrong length for inet_ntoa")
    return packed


def inet_ntoa(packed):
    _check_ipv4(packed)
    return ".".join(str(b) for b in packed)


def inet_aton(ip_string):
    parts = ip_string.split(".")
    if len(parts) != 4:
        raise OSError("illegal IP address string passed to inet_aton")
    out = []
    for p in parts:
        out.append(int(p) & 0xFF)
    return bytes(out)


def inet_ntop(address_family, packed):
    if address_family == AF_INET:
        return inet_ntoa(packed)
    if address_family == AF_INET6:
        # v0: colon-separated hex groups (no :: compression).
        groups = [int.from_bytes(packed[i:i + 2], "big") for i in range(0, 16, 2)]
        return ":".join(format(g, "x") for g in groups)
    raise OSError("unknown address family")


def inet_pton(address_family, ip_string):
    if address_family == AF_INET:
        return inet_aton(ip_string)
    if address_family == AF_INET6:
        out = bytearray(16)
        groups = ip_string.split(":")
        if len(groups) > 8:
            raise OSError("illegal IP address string passed to inet_pton")
        for i, g in enumerate(groups[:8]):
            out[i * 2] = int(g, 16) >> 8
            out[i * 2 + 1] = int(g, 16) & 0xFF
        return bytes(out)
    raise OSError("unknown address family")
