"""antipythany deviation shim — service_identity (v0: verification always
passes; the file:// gate performs no TLS)."""

__version__ = "23.1.0"


class VerificationError(Exception):
    pass


class CertificateError(VerificationError):
    pass


class DNSMismatch(VerificationError):
    pass


class IPAddressMismatch(VerificationError):
    pass


def verify_hostname(connection, hostname):
    return None


def verify_ip_address(connection, ip):
    return None


def extract_ids(connection):
    return []


pyOpenSSL = None
