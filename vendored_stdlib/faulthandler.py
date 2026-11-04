"""faulthandler stub (antipythony v0): the stdlib module dumps native
tracebacks; under our runtime that surface is inert. pytest imports it for
its faulthandler plugin; provide the names it uses as no-ops."""

__all__ = [
    "enable",
    "disable",
    "is_enabled",
    "dump_traceback_later",
    "cancel_dump_traceback_later",
    "dump_traceback",
    "register",
    "unregister",
]


def enable(file=None):
    return None


def disable():
    return None


def is_enabled():
    return False


def dump_traceback_later(timeout, repeat=False, file=None, exit=False):
    return None


def cancel_dump_traceback_later():
    return None


def dump_traceback(file=None, all_threads=True):
    return None


def register(file, all_threads=True, chain=False):
    return None


def unregister(file):
    return None
