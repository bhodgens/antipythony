"""antipythany deviation shim: threading.

The T1 (scrapy file://) gate never spawns threads; twisted's
threadable.synchronize and logging QueuedLogging paths only need the
LOCK TYPES to exist. All primitives are inert (no real locking).
"""

import sys as _sys


class _InertLock:
    def __init__(self, *a, **k):
        pass

    def acquire(self, blocking=True, timeout=-1):
        return True

    def release(self):
        return None

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        return False


def RLock(*a, **k):
    return _InertLock()


def _RLock(*a, **k):
    return _InertLock()


def Lock(*a, **k):
    return _InertLock()


def BoundedSemaphore(*a, **k):
    return _InertLock()


def Semaphore(*a, **k):
    return _InertLock()


def Condition(*a, **k):
    return _InertLock()


def Event(*a, **k):
    return _InertLock()


def Barrier(*a, **k):
    return _InertLock()


def Timer(*a, **k):
    return _InertLock()


def active_count():
    return 1


def current_thread():
    return _sys.modules.get("threading")


def main_thread():
    return current_thread()


def enumerate():
    return []


def local():
    return _InertLock()


def settrace(func):
    pass


def setprofile(func):
    pass


def stack_size(size=None):
    return 0


TIMEOUT_MAX = 4294967.0
