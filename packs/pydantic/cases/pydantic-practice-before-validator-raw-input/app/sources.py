import json as jsonlib

import flask
import httpx
import requests
import yaml
from fastapi import Request
from pydantic import BaseModel, field_validator

from app.services import baskets


# One model per way foreign data reaches model_validate; each validator is unguarded.
class FromLoads(BaseModel):
    x: str

    @field_validator("x", mode="before")
    @classmethod
    def strip_x(cls, v):
        return v.strip()  # caught


class FromRequestJson(BaseModel):
    x: str

    @field_validator("x", mode="before")
    @classmethod
    def strip_x(cls, v):
        return v.strip()  # caught


class FromFlask(BaseModel):
    x: str

    @field_validator("x", mode="before")
    @classmethod
    def strip_x(cls, v):
        return v.strip()  # caught


class FromHttpResponse(BaseModel):
    x: str

    @field_validator("x", mode="before")
    @classmethod
    def strip_x(cls, v):
        return v.strip()  # caught


class FromYamlKey(BaseModel):
    x: str

    @field_validator("x", mode="before")
    @classmethod
    def strip_x(cls, v):
        return v.strip()  # caught


class FromSplat(BaseModel):
    x: str

    @field_validator("x", mode="before")
    @classmethod
    def strip_x(cls, v):
        return v.strip()  # caught


# Parsed from the project's own objects, or from a value whose origin the code does not show: not exposed.
class FromRow(BaseModel):
    x: str

    @field_validator("x", mode="before")
    @classmethod
    def strip_x(cls, v):
        return v.strip()


class FromDump(BaseModel):
    x: str

    @field_validator("x", mode="before")
    @classmethod
    def strip_x(cls, v):
        return v.strip()


class FromUnknown(BaseModel):
    x: str

    @field_validator("x", mode="before")
    @classmethod
    def strip_x(cls, v):
        return v.strip()


def from_loads(raw: str) -> FromLoads:
    data = jsonlib.loads(raw)
    return FromLoads.model_validate(data)


async def from_request(request: Request) -> FromRequestJson:
    return FromRequestJson.model_validate(await request.json())


def from_flask() -> FromFlask:
    return FromFlask.model_validate(flask.request.get_json())


def from_http(url: str) -> list:
    resp = httpx.get(url)
    item = FromHttpResponse.model_validate(resp.json())
    return [item, baskets.validate_json(requests.get(url).text)]


def from_yaml(text: str) -> FromYamlKey:
    return FromYamlKey.model_validate(yaml.safe_load(text)["item"])


def from_splat(raw: bytes) -> FromSplat:
    payload = jsonlib.loads(raw)
    return FromSplat(**payload)


def from_rows(rows) -> list[FromRow]:
    return [FromRow.model_validate(row) for row in rows]


def from_dump(other: FromRow) -> FromDump:
    return FromDump.model_validate(other.model_dump())


def from_unknown(data: dict) -> FromUnknown:
    return FromUnknown.model_validate(data)
