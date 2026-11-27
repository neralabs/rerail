import jwt
from rest_framework_simplejwt.authentication import JWTAuthentication

TRUSTED_ISSUER = "us"


class RoutingAuth(JWTAuthentication):
    # The review's counterexample: foreign issuers go to the next class, the rest to the verifying parent.
    def authenticate(self, request):
        raw = request.headers.get("Authorization", "").split(" ")[-1]
        claims = jwt.decode(raw, options={"verify_signature": False})
        if claims.get("iss") != TRUSTED_ISSUER:
            return None
        return super().authenticate(request)


class ExplicitParentAuth(JWTAuthentication):
    def authenticate(self, request):
        raw = request.headers.get("Authorization", "").split(" ")[-1]
        claims = jwt.decode(raw, options={"verify_signature": False})
        if claims.get("iss") != TRUSTED_ISSUER:
            return None
        return JWTAuthentication.authenticate(self, request)


class ValidatedTokenAuth(JWTAuthentication):
    def authenticate(self, request):
        raw = self.get_raw_token(self.get_header(request))
        claims = jwt.decode(raw, options={"verify_signature": False})
        if claims.get("iss") != TRUSTED_ISSUER:
            return None
        validated = self.get_validated_token(raw)
        return self.get_user(validated), validated


class PeekingAuth(JWTAuthentication):
    # Calls the parent, but not its authentication: the user comes from the unverified claims.
    def authenticate(self, request):
        raw = request.headers.get("Authorization", "").split(" ")[-1]
        claims = jwt.decode(raw, options={"verify_signature": False})  # caught
        return super().get_user(claims), raw
