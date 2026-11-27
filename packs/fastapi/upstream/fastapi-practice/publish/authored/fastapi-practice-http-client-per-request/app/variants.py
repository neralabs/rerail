from functools import lru_cache as memo

import httpx
import httpx as hx
from fastapi import APIRouter, Depends
from httpx import post as http_post

router = APIRouter()


class ApiClient(httpx.AsyncClient):
    pass


# The same constructions under aliases and through a subclass of the run.
@router.get("/v/alias-module")
def alias_module(url: str):
    return hx.get(url).json()  # caught


@router.post("/v/alias-function")
def alias_function(url: str):
    return http_post(url, json={}).json()  # caught


@router.get("/v/subclass")
async def subclass(url: str):
    async with ApiClient() as client:  # caught
        return (await client.get(url)).json()


# A cached dependency under an alias of lru_cache: built once.
@memo
def shared_client() -> httpx.Client:
    return httpx.Client(timeout=5)


@router.get("/v/cached-alias")
def cached_alias(url: str, client: httpx.Client = Depends(shared_client)):
    return client.get(url).json()


# Another library's `get` with the same name: not httpx.
@router.get("/v/other-get")
def other_get(key: str):
    cache = {"a": 1}
    return cache.get(key)
