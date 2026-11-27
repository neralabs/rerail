import jwt
from django.conf import settings
from django.contrib.auth import get_user_model
from rest_framework import authentication

User = get_user_model()
NO_SIGNATURE = {"verify_signature": False}


class BearerAuthentication(authentication.BaseAuthentication):
    def authenticate(self, request):
        header = request.headers.get("Authorization", "")
        if not header.startswith("Bearer "):
            return None
        token = header.split()[1]
        opts = {"verify_signature": False, "verify_exp": True}
        payload = jwt.decode(token, options=opts)  # caught
        return (User.objects.get(pk=payload["user_id"]), token)


class SignedBearerAuthentication(authentication.BaseAuthentication):
    def authenticate(self, request):
        token = request.headers.get("Authorization", "").removeprefix("Bearer ")
        payload = jwt.decode(token, settings.SECRET_KEY, algorithms=["HS256"])
        return (User.objects.get(pk=payload["user_id"]), token)


class JWTBackend:
    def authenticate(self, request, token=None):
        claims = jwt.decode(token, options=NO_SIGNATURE)  # caught
        try:
            return User.objects.get(email=claims["email"])
        except User.DoesNotExist:
            return None


class FormatSniffingAuthentication(authentication.BaseAuthentication):
    def authenticate(self, request):
        token = request.headers.get("Authorization", "").removeprefix("Bearer ")
        try:
            jwt.decode(token, options={"verify_signature": False})
        except jwt.DecodeError:
            return None
        return self.authenticate_signed(token)

    def authenticate_signed(self, token):
        payload = jwt.decode(token, settings.SECRET_KEY, algorithms=["HS256"])
        return (User.objects.get(pk=payload["user_id"]), token)
