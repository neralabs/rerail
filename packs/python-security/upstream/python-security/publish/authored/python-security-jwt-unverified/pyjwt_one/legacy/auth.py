import jwt

from legacy.tokens import SECRET

PUBLIC_KEY = "-----BEGIN PUBLIC KEY-----"


def decode_legacy(token):
    return jwt.decode(token, PUBLIC_KEY)  # caught


def decode_legacy_none(token):
    return jwt.decode(token, SECRET, algorithms=None)  # caught


def decode_legacy_pinned(token):
    return jwt.decode(token, SECRET, algorithms=["HS256"])


def decode_legacy_kwargs(token, **kwargs):
    return jwt.decode(token, SECRET, **kwargs)
