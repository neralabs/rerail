from fastapi import APIRouter
from pydantic import BaseModel, field_validator

from app.routes import router as main_router


# A project router class deriving from FastAPI's: its routes are routes.
class UserAPIRouter(APIRouter):
    pass


user_router = UserAPIRouter(prefix="/users")


# Router classes built by factories (polar's `APIRouter = get_api_router_class(APIRoute)`): by the return annotation,
# or by the class the factory defines and returns.
def router_class_for(route_class: type) -> type[APIRouter]:
    class _Router(APIRouter):
        pass

    return _Router


def plain_router_class():
    class _Plain(APIRouter):
        pass

    return _Plain


AnnotatedRouter = router_class_for(object)
PlainRouter = plain_router_class()
annotated_router = AnnotatedRouter(prefix="/a")
plain_router = PlainRouter(prefix="/p")


class Alias(BaseModel):
    alias: str

    @field_validator("alias", mode="before")
    @classmethod
    def strip_alias(cls, v):
        return v.strip()  # caught


class Handle(BaseModel):
    handle: str

    @field_validator("handle", mode="before")
    @classmethod
    def strip_handle(cls, v):
        return v.strip()  # caught


@annotated_router.post("/alias")
def set_alias(body: Alias) -> None: ...


@plain_router.put("/handle")
def set_handle(body: Handle) -> None: ...


class Nickname(BaseModel):
    nick: str

    @field_validator("nick", mode="before")
    @classmethod
    def strip_nick(cls, v):
        return v.strip()  # caught


class Motto(BaseModel):
    motto: str

    @field_validator("motto", mode="before")
    @classmethod
    def strip_motto(cls, v):
        return v.strip()  # caught


@user_router.post("/nickname")
def set_nickname(body: Nickname) -> None: ...


# The router imported from another module.
@main_router.post("/motto")
def set_motto(body: Motto) -> None: ...


# A look-alike: a cache object with a `get` decorator is no router.
class Cache:
    def get(self, key: str):
        def wrap(fn):
            return fn

        return wrap


cache = Cache()


class CacheKey(BaseModel):
    key: str

    @field_validator("key", mode="before")
    @classmethod
    def strip_key(cls, v):
        return v.strip()


@cache.get("profile")
def cached_profile(key: CacheKey) -> None: ...
