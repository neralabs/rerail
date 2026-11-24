from typing import Any

from pydantic import BaseModel, field_validator, model_validator, root_validator, validator


class Signup(BaseModel):
    email: str
    nickname: str | None = None
    tags: list[str] = []
    country: str = ""
    code: str = ""
    zone: str = ""

    # The annotation does not protect: Pydantic does not check a before validator's argument.
    @field_validator("email", mode="before")
    @classmethod
    def normalise_email(cls, v: str) -> str:
        return v.strip().lower()  # caught

    # Only None is let through; 123 still reaches `.split`.
    @field_validator("nickname", mode="before")
    @classmethod
    def blank_nickname(cls, v):
        if v is None or v == "":
            return None
        return v.split("#")[0]  # caught

    # Iterating an int raises TypeError.
    @field_validator("tags", mode="before")
    @classmethod
    def split_tags(cls, v):
        return [t.strip() for t in v]  # caught

    # A wrap validator gets the raw input too.
    @field_validator("country", mode="wrap")
    @classmethod
    def upper_country(cls, v, handler):
        return handler(v.upper())  # caught

    # A plain validator replaces Pydantic's own validation: nothing else checks the value.
    @field_validator("code", mode="plain")
    @classmethod
    def code_plain(cls, v):
        return v.removeprefix("C-")  # caught

    # Only the first unchecked use of a validator is reported.
    @field_validator("zone", mode="before")
    @staticmethod
    def zone_static(v):
        if len(v) > 3:  # caught
            return v[:3]
        return v


class Address(BaseModel):
    street: str
    city: str = ""

    @model_validator(mode="before")
    @classmethod
    def legacy_keys(cls, data: dict[str, Any]) -> dict[str, Any]:
        if "line1" in data:
            data["street"] = data.pop("line1")  # caught
        return data

    # `data or {}` keeps any truthy non-dict.
    @model_validator(mode="before")
    @classmethod
    def default_city(cls, data):
        data = data or {}
        data.setdefault("city", "Paris")  # caught
        return data


class Order(BaseModel):
    sku: str
    qty: int = 1

    @model_validator(mode="before")
    @classmethod
    def merge(cls, data):
        return {"qty": 1, **data}  # caught

    @model_validator(mode="wrap")
    @classmethod
    def log_input(cls, data, handler):
        for key in data:  # caught
            print(key)
        return handler(data)

    # KeyError does not cover TypeError: `data["sku"]` on a list raises TypeError.
    @field_validator("sku", mode="before")
    @classmethod
    def sku_from_mapping(cls, v):
        try:
            return v["value"]  # caught
        except KeyError:
            return v


class LegacySignup(BaseModel):
    username: str
    email: str = ""

    @validator("username", pre=True)
    def strip_username(cls, v):
        return v.strip()  # caught

    @root_validator(pre=True)
    def drop_empty(cls, values):
        return {k: val for k, val in values.items() if val != ""}  # caught


class Settings(BaseModel):
    debug: bool = False
    level: str = "info"

    # An after model validator gets the instance, even written v1-style as a classmethod.
    @model_validator(mode="after")
    @classmethod
    def check_level(cls, values):
        if values.get("debug"):  # caught
            values.level = "debug"
        return values
