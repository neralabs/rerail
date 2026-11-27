from contextlib import asynccontextmanager

from fastapi import FastAPI


@asynccontextmanager
async def lifespan(app):
    yield


class ShopApp(FastAPI):
    pass


# A subclass of FastAPI built with a lifespan: dead, the other rule's case.
shop = ShopApp(lifespan=lifespan)
shop.add_event_handler("startup", print)


def make_app() -> FastAPI:
    return FastAPI()


# A factory of the run: the literal event is FastAPI's registration, on an app without lifespan (not dead).
made = make_app()
made.on_event("startup")(print)  # caught
