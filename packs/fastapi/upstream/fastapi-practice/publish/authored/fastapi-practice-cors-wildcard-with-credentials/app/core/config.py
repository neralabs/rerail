import os
from functools import lru_cache
from typing import List

from pydantic import Field
from pydantic_settings import BaseSettings

ALL_ORIGINS = ["*"]
ENV_ORIGINS = os.getenv("CORS_ALLOW_ORIGIN", "*").split(";")
ENV_ORIGINS_SAFE = os.getenv("CORS_ALLOW_ORIGIN", "").split(";")
ENV_ORIGIN_STRING = os.environ.get("CORS_ORIGIN", "*")
CREDENTIALS = True
NO_CREDENTIALS = False
PUBLIC_ORIGINS = ["https://app.example.com", "https://admin.example.com"]
ANY_HTTPS = r"https?://.*"


class Settings(BaseSettings):
    CORS_ORIGINS: List[str] = ["*"]
    CORS_ORIGINS_FIELD: list[str] = Field(default=["*"])
    FRONTEND_HOST: str = "http://localhost:5173"
    BACKEND_CORS_ORIGINS: list[str] = []
    ALLOWED: list[str] = PUBLIC_ORIGINS


class ProdSettings(Settings):
    CORS_ORIGINS: List[str] = ["https://app.example.com"]


settings = Settings()
prod_settings = ProdSettings()
explicit_settings = Settings(CORS_ORIGINS=["https://app.example.com"])


@lru_cache
def get_settings() -> Settings:
    return Settings()
