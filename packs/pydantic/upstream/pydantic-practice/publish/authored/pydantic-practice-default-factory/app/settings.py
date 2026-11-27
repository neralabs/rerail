import os
import secrets
from datetime import datetime

from pydantic_settings import BaseSettings


# A settings model is the process's configuration: one value per process is what it means, and a factory would make
# two Settings() instances disagree (a JWT signed with one key and checked with another).
class Settings(BaseSettings):
    SECRET_KEY: str = secrets.token_urlsafe(32)
    STARTED_AT: datetime = datetime.now()
    DATABASE_URL: str = os.getenv("DATABASE_URL", "sqlite://")
    HOME: str = os.environ.get("HOME", "/")


class AppSettings(Settings):
    INSTANCE_TOKEN: str = secrets.token_hex(8)
