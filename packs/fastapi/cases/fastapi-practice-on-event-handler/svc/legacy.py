from fastapi import FastAPI
from starlette.applications import Starlette


async def init_db() -> None: ...


async def close_db() -> None: ...


# No lifespan: the handlers run, but the API is deprecated.
legacy = FastAPI(on_startup=[init_db], on_shutdown=[close_db])  # caught


@legacy.on_event("startup")  # caught
async def seed() -> None: ...


def register(app):
    app.add_event_handler("shutdown", close_db)  # caught


register(legacy)

star = Starlette(on_startup=[init_db])  # caught

# Nothing to register: left alone.
empty = FastAPI(on_startup=[], on_shutdown=None)
