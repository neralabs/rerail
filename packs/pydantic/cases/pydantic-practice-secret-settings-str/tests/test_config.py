from pydantic_settings import BaseSettings


# A settings class a test defines is a fixture of that test.
class TestSettings(BaseSettings):
    SECRET_KEY: str = "test"
    DB_PASSWORD: str = "test"
