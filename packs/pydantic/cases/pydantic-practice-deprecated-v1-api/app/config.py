from pydantic_settings import BaseSettings


class Settings(BaseSettings):
    API_V1_STR: str = "/api/v1"
    USE_V2_FORMAT: bool = True


settings = Settings()
