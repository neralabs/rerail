import jwt
from flask import session


def attach_identity(request, token):
    claims = jwt.decode(token, options={"verify_signature": False})  # caught
    request.user = claims["sub"]


def remember(token):
    payload = jwt.decode(token, options={"verify_signature": False})  # caught
    session["user_id"] = payload["sub"]
