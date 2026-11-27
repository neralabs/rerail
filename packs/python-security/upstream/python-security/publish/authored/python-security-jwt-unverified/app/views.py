import logging

import jwt
from django.http import JsonResponse

from app.models import User
from app.tokens import peek_claims

log = logging.getLogger(__name__)
KEYS = {"https://issuer.example": "secret"}


def whoami(request):
    token = request.headers["Authorization"].split()[1]
    claims = peek_claims(token)  # caught
    user = User.objects.get(pk=claims["sub"])
    return JsonResponse({"email": user.email})


def audit(request):
    token = request.headers["Authorization"].split()[1]
    claims = peek_claims(token)
    log.info("request from issuer %s", claims.get("iss"))
    return JsonResponse({})


def checked_whoami(request):
    token = request.headers["Authorization"].split()[1]
    claims = peek_claims(token)
    key = KEYS[claims["iss"]]
    verified = jwt.decode(token, key, algorithms=["HS256"])
    return JsonResponse({"sub": verified["sub"]})
