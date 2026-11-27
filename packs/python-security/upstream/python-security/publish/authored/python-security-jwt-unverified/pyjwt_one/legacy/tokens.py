import jwt

SECRET = "change-me"


def issue(user_id):
    # PyJWT 1 returns bytes here; PyJWT 2 returns a str, which has no decode().
    return jwt.encode({"sub": user_id}, SECRET, algorithm="HS256").decode("utf-8")
