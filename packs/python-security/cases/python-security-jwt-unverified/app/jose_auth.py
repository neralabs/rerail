from jose import jwt

from app.models import User

SECRET = "change-me"
DONT_VERIFY_AT_HASH = False


def get_current_user(token):
    data = jwt.decode(token, SECRET)  # caught
    return data["email"]


def get_current_user_pkce(token, key):
    jwt_opts = {}
    if DONT_VERIFY_AT_HASH:
        jwt_opts = {"verify_at_hash": False}
    data = jwt.decode(token, key, options=jwt_opts)  # caught
    return data["email"]


def decode_pinned(token):
    return jwt.decode(token, SECRET, algorithms=["HS256"])


def decode_pinned_positional(token):
    return jwt.decode(token, SECRET, ["HS256"])


def decode_explicit_none(token):
    return jwt.decode(token, SECRET, algorithms=None)  # caught


def user_for(token):
    claims = jwt.get_unverified_claims(token)  # caught
    return User.query.filter_by(email=claims["email"]).first()


def issuer_of(token):
    claims = jwt.get_unverified_claims(token)
    return claims["iss"] in {"https://a.example", "https://b.example"}
