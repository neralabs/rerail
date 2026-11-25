from typing import Annotated, Optional, Union

from pydantic import AliasChoices, Field, StrictStr, constr
from pydantic_settings import BaseSettings

from app.core.types import NonEmptyStr, OptionalText, PlainStr


class IntegrationSettings(BaseSettings):
    # The last word part names a secret: upper and snake case, camel case, a trailing number.
    OPENAI_API_KEY_2: str = ""  # caught
    AWS_SECRET_ACCESS_KEY: str = ""  # caught
    secretAccessKey: str = ""  # caught
    PASSWORD_PEPPER: str = ""  # caught
    TOKEN: str = ""  # caught
    jwt_signing_secret: str = ""  # caught
    stripeApiKey: str = ""  # caught
    openaiAPIKey: str = ""  # caught
    privateKey: str = ""  # caught
    apiKey: str = ""  # caught
    githubAccessToken: Optional[str] = None  # caught

    # String types resolved by symbol: pydantic's StrictStr and constr, and module-level aliases of str.
    CONSUL_TOKEN: StrictStr = ""  # caught
    ADMIN_PASSWORD: constr(min_length=12) = "change-me-please"  # caught
    MAIL_PASSWORD: PlainStr = ""  # caught
    BROKER_PASSWORD: Union[None, NonEmptyStr] = None  # caught
    SFTP_PASSWORD: OptionalText = None  # caught

    # The environment variable's name, given as an alias.
    openai: str = Field(default="", alias="OPENAI_API_KEY")  # caught
    stripe: str = Field(default="", validation_alias=AliasChoices("STRIPE_KEY", "STRIPE_SECRET_KEY"))  # caught
    oidc: Annotated[str, Field(alias="OIDC_CLIENT_SECRET")] = ""  # caught
    issuer: str = Field(default="", validation_alias=AliasChoices("TOKEN_ISSUER", "OIDC_ISSUER"))

    # The name mentions a secret but ends in what the value is: an audience, an issuer, a scheme, a cookie name.
    JWT_TOKEN_AUDIENCE: str = "api"
    TOKEN_ISSUER: str = "https://auth.example.com"
    PASSWORD_HASH_SCHEME: str = "argon2"
    AUTH_TOKEN_COOKIE: str = "access_token"
    TOKEN_ENDPOINT: str = "/oauth/token"
    GITHUB_TOKEN_SCOPES: str = "repo"
    PASSWORD_RESET_TEMPLATE: str = "reset.html"
    SECRET_BACKEND: str = "vault"
    API_KEY_LOCATION: str = "header"
    accessTokenAudience: str = "api"
    apiKeyHeader: str = "X-API-Key"
    tokenUrl: str = ""
    PUBLIC_KEY: str = ""
    CACHE_KEY: str = "app"

    # An access key id: the public half of a pair whose secret half is caught.
    S3_ACCESS_KEY: str | None = None
    minioAccessKey: str = ""
    S3_SECRET_KEY: str | None = None  # caught

    # Gaps: an abbreviation, and keys without a secret word before them.
    DB_PASS: str = ""
    FERNET_KEY: str = ""
    OPENAI_KEY: str = ""
