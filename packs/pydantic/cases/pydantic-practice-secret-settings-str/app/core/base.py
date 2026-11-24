from typing import Optional

from pydantic import BaseModel
from pydantic_settings import BaseSettings


# A settings base in another module: its fields are reported once, on this line, for every settings class that
# inherits them.
class DatabaseSettings(BaseSettings):
    POSTGRES_USER: str = "app"
    POSTGRES_PASSWORD: str = "app"  # caught
    POSTGRES_HOST: str = "localhost"


# A plain BaseModel mixin: its fields become the settings class's fields when a settings class derives from it.
class RedisMixin(BaseModel):
    REDIS_HOST: str = "localhost"
    REDIS_PASSWORD: Optional[str] = None  # caught


# A plain model a settings class holds as a field (nested settings, `SMTP__PASSWORD` in the environment): the
# parent's repr and dump print it too.
class SmtpConfig(BaseModel):
    host: str = "localhost"
    user: str | None = None
    password: str | None = None  # caught
    password_min_length: int = 8


# The id of the current secret, and the secrets themselves as a dict: neither a plain-string secret.
class HashSettings(BaseSettings):
    CURRENT_HASH_SECRET_ID: str | None = None
    HASH_SECRETS: dict[str, str] = {}
