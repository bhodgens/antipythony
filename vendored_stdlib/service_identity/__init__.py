# antipythony shim — service_identity (twisted _sslverify chain; v0 constant/method surfaces only)
from service_identity.pyopenssl import (
    VerificationError,
    CertificateError,
    extract_from_certificate,
    extract_from_ssl_socket,
    verify_hostname,
    verify_ip_address,
)
