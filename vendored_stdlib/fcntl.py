"""antipythony deviation shim: minimal fcntl surface (DEVIATIONS.toml).
The file:// gate never performs real descriptor control; twisted's _FDWaker
calls fcntl.fcntl() at import to set non-blocking/close-on-exec flags."""
F_GETFL = 3
F_SETFL = 4
F_GETFD = 1
F_SETFD = 2
FD_CLOEXEC = 1
O_NONBLOCK = 0o4000
O_NDELAY = 0o4000
O_APPEND = 0o2000
O_RDWR = 0o2
O_RDONLY = 0o0
O_WRONLY = 0o1
O_CREAT = 0o100
O_EXCL = 0o200
O_TRUNC = 0o1000
LOCK_EX = 2
LOCK_SH = 1
LOCK_UN = 8
LOCK_NB = 4


def fcntl(fd, cmd, arg=0):
    # Inert: the gate's synthetic fds have no real flags.
    return 0


def ioctl(fd, request, arg=0, mutate_flag=True):
    raise OSError("ioctl is not supported in this runtime")


def flock(fd, operation):
    return None


def openpty():
    raise OSError("openpty is not supported in this runtime")


def lockf(fd, cmd, len, start, whence):
    return None
