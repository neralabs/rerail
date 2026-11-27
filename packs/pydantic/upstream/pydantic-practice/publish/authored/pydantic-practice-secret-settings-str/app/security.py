import hmac
from hashlib import sha256

from app.core.config import settings


def sign(payload: bytes) -> str:
    return hmac.new(settings.SECRET_KEY.encode(), payload, sha256).hexdigest()


def database_dsn() -> str:
    return f"postgresql://{settings.POSTGRES_USER}:{settings.POSTGRES_PASSWORD}@{settings.POSTGRES_HOST}/app"


def jwt_key() -> str:
    return settings.SECRET_KEY
