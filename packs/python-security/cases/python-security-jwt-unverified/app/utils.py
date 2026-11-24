import logging
from datetime import datetime, timezone

import jwt
from authlib.jose import jwt as authlib_jwt

log = logging.getLogger(__name__)
API_KEYS = {"svc": "secret"}


def is_jwt_formatted(token: str) -> bool:
    if not token:
        return False
    try:
        jwt.decode(token, options={"verify_signature": False})
    except jwt.PyJWTError:
        return False
    return True


def jwt_expiry(value: str):
    claims = jwt.decode(value, options={"verify_signature": False, "verify_exp": False})
    exp = claims.get("exp")
    if not isinstance(exp, (int, float)):
        return None
    return datetime.fromtimestamp(exp, tz=timezone.utc)


def log_token(token):
    claims = jwt.decode(token, options={"verify_signature": False})
    log.info("token for %s issued by %s", claims.get("sub"), claims.get("iss"))


def is_service_token(token):
    payload = jwt.decode(token, options={"verify_signature": False})
    return payload.get("owner") == "service"


def verify_api_token(token):
    unverified = jwt.decode(token, options={"verify_signature": False})
    key = API_KEYS[unverified["iss"]]
    return jwt.decode(token, key, algorithms=["HS256"])


def get_current_user_kind(token):
    try:
        jwt.decode(token, options={"verify_signature": False})
    except jwt.DecodeError:
        return "api-key"
    return "jwt"


def decode_authlib(token, key):
    return authlib_jwt.decode(token, key)
