# antipythany deviation shim (see DEVIATIONS.toml): CPython's urllib/request.py
# pulls socket/email/http.client (C and heavy stdlib surfaces this runtime does
# not serve yet). The T1 scrapy chain (w3lib.url) only needs the path conversion
# helpers; they are copied verbatim from CPython 3.14 urllib/request.py
# (posixpath branch, PSF-2.0).

from urllib.parse import (
    uses_netloc, uses_relative, uses_params,
)  # re-exported below for getproxies-adjacent consumers

__all__ = ["pathname2url", "url2pathname"]

# The same protective quotes used by CPython's posix variant.
from urllib.parse import quote as _quote, unquote as _unquote


def pathname2url(pathname):
    """OS-specific conversion from a file system path to a relative URL.

    This doesn't produce a complete URL. The return value will not
    include a 'file://' prefix.
    """
    # Don't leave the leading '/' on the filename as _quote would
    # otherwise happily quote it.
    if pathname[:2] == "//":
        return _quote(pathname[2:], "/:")
    return _quote(pathname, "/:")


def url2pathname(pathname):
    """OS-specific conversion from a relative URL to a file system path;
    this doesn't produce a complete URL."""
    return _unquote(pathname)
