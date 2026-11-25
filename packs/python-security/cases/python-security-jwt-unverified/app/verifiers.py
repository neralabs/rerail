import jwt

from app.models import User

SECRET = "change-me"


def load_verified(token, key):
    return jwt.decode(token, key, algorithms=["RS256"])


def _decode_signed(token):
    return jwt.decode(token, SECRET, algorithms=["HS256"])


def load_user(token):
    payload = _decode_signed(token)
    return User.objects.get(pk=payload["sub"])


def audit_token(token):
    return len(token)
