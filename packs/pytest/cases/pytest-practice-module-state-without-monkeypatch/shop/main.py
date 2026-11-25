from fastapi import FastAPI
from pydantic_settings import BaseSettings


class Settings(BaseSettings):
    debug: bool = False


settings = Settings()
app = FastAPI()
_ENGINE_CACHE = {}
TASKS = {}


def get_db():
    return None


class FxaView:
    keys = None
