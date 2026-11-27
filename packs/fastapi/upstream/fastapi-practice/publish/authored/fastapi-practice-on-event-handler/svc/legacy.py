from fastapi import FastAPI
from starlette.applications import Starlette


async def init_db() -> None: ...


async def close_db() -> None: ...


# No lifespan: the handlers run, so none is reported here.
legacy = FastAPI(on_startup=[init_db], on_shutdown=[close_db])


@legacy.on_event("startup")
async def seed() -> None: ...


def register(app):
    app.add_event_handler("shutdown", close_db)


register(legacy)

star = Starlette(on_startup=[init_db])

# Nothing to register: left alone.
empty = FastAPI(on_startup=[], on_shutdown=None)
