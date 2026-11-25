import jwt
from email_validator import validate_email
from fastapi import Depends
from fastapi.security import HTTPBearer
from google.auth.transport import requests as google_requests
from google.oauth2 import id_token

from app.models import User
from app.verifiers import audit_token, load_user, load_verified

bearer = HTTPBearer()
KEYS = {}
CLIENT_ID = "client"


def get_current_user(creds=Depends(bearer)):
    # The review's counterexample: peeked as `token`, verified as `creds.credentials`.
    token = creds.credentials
    header = jwt.get_unverified_header(token)
    claims = jwt.decode(token, options={"verify_signature": False})
    key = KEYS[claims["iss"]][header["kid"]]
    payload = jwt.decode(creds.credentials, key, algorithms=["RS256"])
    return User.objects.get(pk=payload["sub"])


class TokenAuth:
    def authenticate(self, request):
        # Peeked through one attribute, verified through another that holds the same token.
        claims = jwt.decode(request.auth, options={"verify_signature": False})
        self.raw = request.auth
        payload = jwt.decode(self.raw, KEYS[claims["iss"]], algorithms=["HS256"])
        return User.objects.get(pk=payload["sub"]), None


def current_user_from_helper(creds=Depends(bearer)):
    # Verified by a function of the run, given the expression the peeked name was bound to.
    raw = creds.credentials
    claims = jwt.decode(raw, options={"verify_signature": False})
    payload = load_verified(creds.credentials, KEYS[claims["iss"]])
    return User.objects.get(pk=payload["sub"])


def current_user_two_levels(creds=Depends(bearer)):
    # Verified two calls down: load_user -> _decode_signed.
    token = creds.credentials
    claims = jwt.decode(token, options={"verify_signature": False})
    if claims.get("typ") != "access":
        return None
    return load_user(token)


def current_user_google(creds=Depends(bearer)):
    # Verified by a library call named verify*, given the same token.
    token = creds.credentials
    claims = jwt.decode(token, options={"verify_signature": False})
    if claims.get("iss") == "accounts.google.com":
        id_token.verify_oauth2_token(creds.credentials, google_requests.Request(), CLIENT_ID)
    return User.objects.get(email=claims["email"])


def current_user_audited(creds=Depends(bearer)):
    # The helper it hands the token to does not verify it.
    token = creds.credentials
    claims = jwt.decode(token, options={"verify_signature": False})  # caught
    audit_token(token)
    return User.objects.get(pk=claims["sub"])


def current_user_validates_claim(creds=Depends(bearer)):
    # A validator given a claim, not the token.
    claims = jwt.decode(creds.credentials, options={"verify_signature": False})  # caught
    validate_email(claims["email"])
    return User.objects.get(email=claims["email"])


def current_user_bound_after(creds=Depends(bearer)):
    # Peeked by its expression, verified by a function of the run through a name bound to it.
    claims = jwt.decode(creds.credentials, options={"verify_signature": False})
    token = creds.credentials
    payload = load_verified(token, KEYS[claims["iss"]])
    return User.objects.get(pk=payload["sub"])
