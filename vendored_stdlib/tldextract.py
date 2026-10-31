"""tldextract — v0 shim (antipythany wave-110).

scrapy's cookies middleware instantiates TLDExtract at import time to split
URL hostnames. v0 provides the class with a pure-suffix registrar heuristic:
it splits the hostname on the last two labels unless the TLD is a known
second-level suffix (co.uk-style list kept minimal). Sufficient for the
file:// crawl gate, which never issues cookie-bearing HTTP requests.
"""


class ExtractResult(tuple):
    """(subdomain, domain, suffix) named-tuple-alike."""

    def __new__(cls, subdomain, domain, suffix):
        return tuple.__new__(cls, (subdomain, domain, suffix))

    @property
    def subdomain(self):
        return self[0]

    @property
    def domain(self):
        return self[1]

    @property
    def suffix(self):
        return self[2]

    @property
    def registered_domain(self):
        return ".".join(p for p in (self[1], self[2]) if p)

    @property
    def fqdn(self):
        return ".".join(p for p in self if p)

    @property
    def ipv4(self):
        return None


# Minimal second-level suffix set (co.uk, com.au, ...) — extend on demand.
_SECOND_LEVEL = {
    "co.uk", "org.uk", "ac.uk", "gov.uk",
    "com.au", "net.au", "org.au", "edu.au",
    "co.jp", "or.jp", "ne.jp",
    "com.br", "com.cn", "com.mx", "co.in", "co.nz",
}


class TLDExtract:
    def __init__(self, *a, **kw):
        pass

    def __call__(self, url, **kw):
        host = url
        if "://" in host:
            host = host.split("://", 1)[1]
        host = host.split("/", 1)[0]
        host = host.split("@", 1)[-1].split(":", 1)[0]
        if host.endswith("."):
            host = host[:-1]
        # IPv4/numeric host: no split.
        if host.replace(".", "").isdigit():
            return ExtractResult("", host, "")
        labels = host.split(".") if host else []
        if len(labels) <= 2:
            return ExtractResult("", host if host else "", "")
        two = ".".join(labels[-2:])
        if two in _SECOND_LEVEL and len(labels) >= 3:
            return ExtractResult(
                ".".join(labels[:-3]), labels[-3], two
            )
        return ExtractResult(
            ".".join(labels[:-2]), labels[-2], labels[-1]
        )

    # real API surface used by scrapy elsewhere
    def update(self, *a, **kw):
        pass


__all__ = ["TLDExtract", "ExtractResult"]
