# antipythony deviation shim (see DEVIATIONS.toml): CPython's urllib/request.py
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


def _parse_proxy(proxy):
    """Return (scheme, user, password, host/port) given a URL or an authority.

    antipythony wave-110: ported from CPython 3.14 urllib.request with the
    _splittype/_splituser/_splitpasswd helpers inlined (our vendored copy
    predates those helpers).
    """
    # inline _splittype: split "scheme:rest"
    if ":" in proxy:
        i = proxy.index(":")
        scheme, r_scheme = proxy[:i], proxy[i + 1:]
    else:
        scheme, r_scheme = None, proxy
    if not r_scheme.startswith("/"):
        # authority
        scheme = None
        authority = proxy
    else:
        # URL
        if not r_scheme.startswith("//"):
            raise ValueError("proxy URL with no authority: %r" % proxy)
        if "@" in r_scheme:
            host_separator = r_scheme.find("@")
            end = r_scheme.find("/", host_separator)
        else:
            end = r_scheme.find("/", 2)
        if end == -1:
            end = None
        authority = r_scheme[2:end]
    # inline _splituser: split "user:password@host"
    if "@" in authority:
        userinfo, hostport = authority.rsplit("@", 1)
    else:
        userinfo, hostport = None, authority
    if userinfo is not None:
        # inline _splitpasswd
        if ":" in userinfo:
            user, password = userinfo.split(":", 1)
        else:
            user, password = userinfo, None
    else:
        user = password = None
    return scheme, user, password, hostport


def getproxies():
    """antipythony wave-110: env-var proxies only (no system proxy lookup).

    Matches CPython getproxies_environment(): scans <scheme>_proxy for every
    scheme (lowercase + uppercase), returns a {scheme: url} dict; "no_proxy"
    is honored by proxy_bypass consumers, not here.
    """
    proxies = {}
    import os as _os
    for name, value in _os.environ.items():
        name = name.lower()
        if value and name.endswith("_proxy") and name != "no_proxy":
            proxies[name[:-6]] = value
    # CPython also exposes plain "http_proxy" via uppercase-only check; keep
    # both cases covered by the loop above.
    return proxies


def proxy_bypass(host):
    """v0: never bypass (no system proxy config on this runtime)."""
    return 0


def proxy_bypass_environment(host, proxies=None):
    return 0
