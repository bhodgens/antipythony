"""sqlite3 - v0 import-success shim (antipythony).

scrapy's disk queues import sqlite3 for persistence backends; the file://
crawl gate never exercises them (JOBDIR unset -> memory queues). Real
sqlite3.connect raises NotImplementedError lazily.
"""
__all__ = ["connect", "Error", "OperationalError", "DatabaseError",
           "IntegrityError", "ProgrammingError", "Row", "Cursor",
           "Connection", "paramstyle", "sqlite_version"]


class Error(Exception):
    pass


class DatabaseError(Error):
    pass


class OperationalError(DatabaseError):
    pass


class IntegrityError(DatabaseError):
    pass


class ProgrammingError(DatabaseError):
    pass


class Cursor:
    def __init__(self, conn):
        self.connection = conn

    def execute(self, *a, **k):
        raise NotImplementedError("sqlite3 v0 shim")

    def executemany(self, *a, **k):
        raise NotImplementedError("sqlite3 v0 shim")

    def close(self):
        pass


class Connection:
    def __init__(self, *a, **k):
        pass

    def cursor(self):
        return Cursor(self)

    def commit(self):
        pass

    def rollback(self):
        pass

    def close(self):
        pass

    def execute(self, *a, **k):
        raise NotImplementedError("sqlite3 v0 shim")


def connect(*a, **k):
    raise NotImplementedError("sqlite3 v0 shim: database persistence not supported")


paramstyle = "qmark"
sqlite_version = "0-shim"
