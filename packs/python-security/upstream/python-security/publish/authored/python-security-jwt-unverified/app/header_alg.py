import jwt
import requests
from flask import abort, g, request

from app.tokens import fetch_token_header

ALLOWED_ALGORITHMS = {"RS256", "ES256"}
SECRETS = ["one", "two"]


def validate_id_token(id_token, client_id, jwks_url):
    header_data = fetch_token_header(id_token)
    for key in requests.get(jwks_url, timeout=5).json()["keys"]:
        if key["kid"] == header_data["kid"]:
            secret = key["pem"]
            algo = header_data["alg"]
            break
    else:
        return None
    return jwt.decode(id_token, secret, algorithms=[algo], audience=client_id)  # caught


def validate_inline(id_token, secret):
    return jwt.decode(id_token, secret, algorithms=[jwt.get_unverified_header(id_token)["alg"]])  # caught


def validate_checked(id_token, secret):
    header = jwt.get_unverified_header(id_token)
    if header["alg"] not in ALLOWED_ALGORITHMS:
        abort(401)
    return jwt.decode(id_token, secret, algorithms=[header["alg"]])


def validate_checked_name(id_token, secret):
    alg = jwt.get_unverified_header(id_token).get("alg")
    if alg not in ALLOWED_ALGORITHMS:
        abort(401)
    return jwt.decode(id_token, secret, algorithms=[alg])


def validate_by_kid(id_token, keys):
    kid = jwt.get_unverified_header(id_token)["kid"]
    return jwt.decode(id_token, keys[kid], algorithms=["RS256"])


def decode_with_secrets(token, secrets, algorithms):
    for secret in secrets:
        try:
            return jwt.decode(token, secret, algorithms=algorithms)  # caught
        except jwt.InvalidTokenError:
            continue
    return None


def login_required_view():
    token = request.headers["Authorization"].split()[1]
    header = jwt.get_unverified_header(token)
    g.payload = decode_with_secrets(token, SECRETS, algorithms=[header["alg"]])
    g.pinned = decode_with_secrets(token, SECRETS, algorithms=["HS256"])
