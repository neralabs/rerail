from typing import Annotated

from pydantic import (
    BaseModel,
    BeforeValidator,
    PlainValidator,
    WrapValidator,
    field_validator,
    model_validator,
)

from app.normalizers import Slug, normalize_claim_code, trim_internal, upper_if_str


def strip_wrapped(value, handler):
    return handler(value.strip())  # caught


def plain_int(value):
    return int(value.replace(",", ""))  # caught


class ClaimConfirm(BaseModel):
    code: Annotated[str, BeforeValidator(normalize_claim_code)]
    country: Annotated[str, BeforeValidator(upper_if_str)] = ""
    slug: Slug = ""
    title: Annotated[str, BeforeValidator(lambda v: v.strip().lower())] = ""  # caught
    name: Annotated[str, WrapValidator(strip_wrapped)] = ""
    amount: Annotated[int, PlainValidator(plain_int)] = 0


# Validators named by the review: a positive check that exits leaves every other type (an int reaches `.split`),
# and `in` on the raw input.
class Tagged(BaseModel):
    tags: list[str]

    @field_validator("tags", mode="before")
    @classmethod
    def split(cls, v):
        if isinstance(v, list):
            return v
        return v.split(",")  # caught


class Contains(BaseModel):
    meta: dict

    @model_validator(mode="before")
    @classmethod
    def need_key(cls, data):
        if "kind" not in data:  # caught
            raise ValueError("kind required")
        return data


# Built only from another model's dump: the project's own values.
class Internal(BaseModel):
    payload: dict
    note: Annotated[str, BeforeValidator(trim_internal)] = ""

    @model_validator(mode="before")
    @classmethod
    def fill(cls, data):
        data.setdefault("payload", {})
        return data


# A query model FastAPI fills from typed query values (app/routes.py, `Depends()`): left alone.
class ItemQuery(BaseModel):
    q: str | None = None

    @field_validator("q", mode="before")
    @classmethod
    def strip_q(cls, v):
        return v.strip() if v else v


class PageQuery(BaseModel):
    cursor: str = ""

    @field_validator("cursor", mode="before")
    @classmethod
    def strip_cursor(cls, v):
        return v.strip()


def build(src: ClaimConfirm) -> Internal:
    return Internal.model_validate(src.model_dump())
