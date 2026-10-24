"""antipythany deviation shim — service_identity.hazmat (v0 inert)."""


class DNS_ID:
    def __init__(self, hostname, *, accept_wildcards=True):
        self.hostname = hostname

    def verify(self, connection):
        return None


class IPAddress_ID:
    def __init__(self, ip):
        self.ip = ip

    def verify(self, connection):
        return None


class Service_ID:
    def __init__(self, hostname):
        self.hostname = hostname


class DNSMismatch(VerificationError if False else object):
    pass


class _VerificationError(Exception):
    pass


def verify_service_identity(service_ids, cert_ids, optional_ids=None):
    return None


def verify_certificate_identity(cert, hostname):
    return []


def dns_names_match(cert_names, hostname):
    return True
