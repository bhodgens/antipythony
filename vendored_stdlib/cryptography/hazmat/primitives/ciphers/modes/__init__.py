"""antipythony deviation shim — ciphers.modes (v0).

Package so BOTH forms work:
  from cryptography.hazmat.primitives.ciphers import modes
  from cryptography.hazmat.primitives.ciphers.modes import CBC, GCM
"""

from cryptography.hazmat.primitives.ciphers.modes._mod import (
    BlockCipherAlgorithm,
    CBC,
    CFB,
    CFB8,
    CTR,
    CipherAlgorithm,
    ECB,
    GCM,
    Mode,
    ModeWithAuthenticationTag,
    ModeWithInitializationVector,
    ModeWithNonce,
    ModeWithTweak,
    OFB,
    XTS,
)
