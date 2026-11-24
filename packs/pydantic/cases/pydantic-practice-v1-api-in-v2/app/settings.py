import pydantic
from pydantic import BaseSettings  # caught

try:
    from pydantic_settings import BaseSettings as NewSettings
except ImportError:
    from pydantic import BaseSettings as NewSettings


class Settings(BaseSettings):
    database_url: str = "sqlite://"


class Other(pydantic.BaseSettings):  # caught
    debug: bool = False
