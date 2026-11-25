import jwt

from app.models import User


def lookup_by_repo(repo, token):
    # A repository's get outside an authentication hook.
    claims = jwt.decode(token, options={"verify_signature": False})  # caught
    return repo.get(claims["sub"])


class BearerAuth:
    def authenticate(self, request):
        token = request.headers["Authorization"][7:]
        payload = jwt.decode(token, options={"verify_signature": False})  # caught
        return User.objects.get(pk=int(payload["user_id"])), token
