"""secrets — stdlib-compatible shim (antipythony vendored).

CPython 3.14 secrets is a thin wrapper over random.SystemRandom. Provide the
common surface used by third-party packages (token_hex, token_bytes,
token_urlsafe, compare_digest, choice, randbelow).
"""
import hmac as _hmac
import os as _os

rng = None  # stdlib exposes SystemRandom here; v0 shim: os.urandom-backed


def token_bytes(nbytes=None):
    if nbytes is None:
        nbytes = 32
    return _os.urandom(nbytes)


def token_hex(nbytes=None):
    return token_bytes(nbytes).hex()


def token_urlsafe(nbytes=None):
    import base64
    tok = token_bytes(nbytes)
    return base64.urlsafe_b64encode(tok).rstrip(b"=").decode("ascii")


def compare_digest(a, b):
    return _hmac.compare_digest(a, b)


def choice(sequence):
    import random as _random
    return _random.choice(sequence)


def randbelow(exclusive_upper_bound):
    import random as _random
    return _random.randrange(exclusive_upper_bound)


DEFAULT_ENTROPY = 32
