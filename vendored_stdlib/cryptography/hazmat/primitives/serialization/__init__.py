"""antipythony deviation shim — cryptography.hazmat.primitives.serialization (v0).

Package so BOTH forms work:
  from cryptography.hazmat.primitives import serialization
  from cryptography.hazmat.primitives.serialization import load_pem_private_key
"""

from cryptography.hazmat.primitives.serialization._mod import (
    BestAvailableEncryption,
    Encoding,
    KeySerializationEncryption,
    NoEncryption,
    ParameterFormat,
    PrivateFormat,
    PublicFormat,
    SSHCertPrivateKeyTypes,
    SSHCertPublicKeyTypes,
    SSHCertificate,
    SSHCertificateBuilder,
    SSHCertificateType,
    SSHPrivateKeyTypes,
    SSHPublicKeyTypes,
    load_der_parameters,
    load_der_private_key,
    load_der_public_key,
    load_pem_parameters,
    load_pem_private_key,
    load_pem_public_key,
    load_ssh_private_key,
    load_ssh_public_identity,
    load_ssh_public_key,
    ssh_key_fingerprint,
)
