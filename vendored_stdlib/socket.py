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


def getaddrinfo(*args, **kwargs):
    return []


def create_connection(*args, **kwargs):
    raise NotImplementedError("network sockets are not supported in this runtime")


def socket(*args, **kwargs):
    raise NotImplementedError("network sockets are not supported in this runtime")
