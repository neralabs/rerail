import jwt

PUBLIC_KEY = "-----BEGIN PUBLIC KEY-----"


def decode(token):
    return jwt.decode(token, PUBLIC_KEY)  # caught


def decode_pinned(token):
    return jwt.decode(token, PUBLIC_KEY, algorithms=["RS256"])
