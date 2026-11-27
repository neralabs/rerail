from pydantic.v1 import BaseSettings


# A pydantic.v1 settings class: lib/pydantic leaves v1 models out.
class LegacySettings(BaseSettings):
    SECRET_KEY: str = ""
