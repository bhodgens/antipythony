# antipythany deviation shim (DEVIATIONS.toml): select wraps the _select C
# extension (docs/compat-contract.md C-07). The T1 gate crawls file:// URLs
# and never performs I/O multiplexing; import-time names only.
class error(OSError):
    pass


select = __import__(__name__)  # module self-reference for attribute reads

POLLIN = 1
POLLOUT = 4
POLLPRI = 2
POLLERR = 8
POLLHUP = 16
POLLNVAL = 32

# selectors.py probes these at import on the current platform.
EPOLLIN = 1
EPOLLOUT = 4
EPOLLPRI = 2
EPOLLERR = 8
EPOLLHUP = 16
EPOLLRDHUP = 8192
EPOLLET = -2147483648
EPOLLONESHOT = 1073741824
EPOLLEXCLUSIVE = 268435456
EPOLL_CLOEXEC = 524288
KQ_FILTER_READ = -1
KQ_FILTER_WRITE = -2
KQ_EV_ADD = 1
KQ_EV_DELETE = 2
KQ_EV_ENABLE = 4
KQ_EV_ONESHOT = 16
KQ_EV_CLEAR = 32
KQ_NOTE_DELETE = 1
KQ_NOTE_WRITE = 2
HAS_EPOLL = False
HAS_KQUEUE = False
devpoll = None
kqueue = None
epoll = None


def select(*args, **kwargs):
    raise NotImplementedError("select requires the _select C extension")


class poll:
    def __init__(self, *args, **kwargs):
        pass

    def register(self, *a, **k):
        pass

    def unregister(self, *a, **k):
        pass

    def modify(self, *a, **k):
        pass

    def poll(self, *a, **k):
        return []


def epoll(*args, **kwargs):
    raise NotImplementedError("epoll requires the _select C extension")


def kqueue(*args, **kwargs):
    raise NotImplementedError("kqueue requires the _select C extension")
