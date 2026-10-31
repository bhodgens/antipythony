"""service_identity.exceptions — v0 shim (antipythany wave-110).

Real exception classes so `except` clauses and imports in twisted's TLS
verification path resolve. No actual verification is performed in v0.
"""


class ServiceIdentityError(Exception):
    """Base for all service_identity errors (real API)."""


class VerificationError(ServiceIdentityError):
    """Raised when identity verification fails (real API)."""


class CertificateError(ServiceIdentityError):
    """Raised when a certificate cannot be parsed/used (real API)."""


class DNSMismatch(VerificationError):
    def __init__(self, mismatched_id=""):
        super().__init__(str(mismatched_id))
        self.mismatched_id = mismatched_id


class ServiceProviderMismatch(VerificationError):
    pass


__all__ = [
    "ServiceIdentityError",
    "VerificationError",
    "CertificateError",
    "DNSMismatch",
    "ServiceProviderMismatch",
]
