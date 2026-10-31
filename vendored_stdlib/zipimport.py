"""zipimport — minimal surface (antipythany vendored shim).

CPython exposes zipimport unconditionally. Our runtime reads .py sources
directly from the filesystem, so zip-archive importing is out of scope; the
module exists so `import zipimport` and feature probes (pkgutil, PyInstaller
-style checks) succeed. `zipimporter(path)` raises ImportError like CPython
does for nonexistent archives.
"""


class ZipImportError(ImportError):
    """Raised when a zip archive cannot be used for importing."""


class zipimporter:
    """v0: constructing against any path raises ZipImportError (no archive
    support). CPython only succeeds when `path` names a valid zip with
    importable contents."""

    def __init__(self, path):
        raise ZipImportError("zipimport: no zip archive support in antipythany v0")

    # CPython class attribute: archive inspection entry points exist even on
    # instances that failed; provide the names for introspection probes.
    archive = None
    prefix = ""


def _zip_searchorder():
    return []


# pkgutil/probes check the callable exists.
find_loader = None
exec_zipimport = None


__all__ = ["zipimporter", "ZipImportError"]
