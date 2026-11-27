from typing import Annotated, ClassVar, Literal, Optional, Union

from pydantic import Field, PlainSerializer, SecretStr, field_serializer
from pydantic_settings import BaseSettings, SettingsConfigDict

from app.core.base import DatabaseSettings, RedisMixin, SmtpConfig
from app.core.types import MaskedStr, VaultSecret


class Settings(DatabaseSettings, RedisMixin):
    model_config = SettingsConfigDict(env_file=".env", env_nested_delimiter="__")

    API_V1_STR: str = "/api/v1"
    SECRET_KEY: str  # caught
    FIRST_SUPERUSER_PASSWORD: str = "changethis"  # caught
    SMTP_PASSWORD: str | None = None  # caught
    STRIPE_WEBHOOK_SECRET: Optional[str] = None  # caught
    OPENAI_API_KEY: Annotated[str, Field(min_length=20)]  # caught
    GITHUB_CLIENT_SECRET: str = Field(default="", description="OAuth app secret")  # caught
    SENDGRID_APIKEY: Union[str, None] = None  # caught
    GITHUB_APP_PRIVATE_KEY: Optional[Annotated[str, Field(min_length=1)]] = None  # caught
    GOOGLE_CREDENTIALS: None | str = None  # caught
    LDAP_PASSWD: "str" = ""
    slackBotToken: str = ""  # caught
    smtp: SmtpConfig = SmtpConfig()

    # Not secrets, or not plain strings.
    ACCESS_TOKEN_EXPIRE_MINUTES: int = 60 * 24 * 8
    PASSWORD_MIN_LENGTH: int = 8
    TOKEN_TYPE: str = "bearer"
    JWT_SECRET_ALGORITHM: str = "HS256"
    JWT_SIGNING_KEY_ID: str = "k1"
    AUTH_SECRET_HEADER: str = "X-Proxy-Secret"
    TOKEN_URL: str = "https://example.com/oauth/token"
    SECRET_KEY_FILE: str = "/run/secrets/key"
    SECRETS_DIR: str = "/run/secrets"
    API_KEY_NAME: str = "X-API-Key"
    API_KEY_PREFIX: str = "sk_"
    AWS_SECRET_ARN: str = ""
    SESSION_TOKEN_TTL: str = "1h"
    REFRESH_TOKEN_LIFETIME: str = "30d"
    API_KEY_ENABLED: bool = False
    TOKENIZER: str = "cl100k_base"
    PASSWORDLESS_LOGIN: bool = False
    PASSWORD_HASH_ROUNDS: int = 12
    SECRET_KEYS: list[str] = []
    TOKEN_SCOPES: dict[str, str] = {}
    TOKEN_MODE: Literal["jwt", "opaque"] = "jwt"
    DATABASE_URL: str = "postgresql://app:app@localhost/app"

    # The safe types.
    JWT_SECRET: SecretStr = SecretStr("")
    ADMIN_PASSWORD: SecretStr | None = None
    VAULT_TOKEN: VaultSecret | None = None

    # Masked by the author: a serializer, `repr=False`, a project alias.
    WEBHOOK_SECRET: Annotated[str, PlainSerializer(lambda v: "*****", return_type=str)] = ""
    SENTRY_AUTH_TOKEN: str = Field(default="", repr=False)
    SMTP_USER_PASSWORD: MaskedStr = ""
    SESSION_SECRET: str = ""

    # Class attributes and private attributes are not fields.
    DEFAULT_PASSWORD: ClassVar[str] = "changeme"
    _bootstrap_token: str = ""

    @field_serializer("SESSION_SECRET")
    def _mask(self, v: str) -> str:
        return "*****"


class WorkerSettings(Settings):
    CELERY_BROKER_PASSWORD: str = ""  # caught


settings = Settings()  # type: ignore[call-arg]
