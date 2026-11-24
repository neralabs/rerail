import logging
import warnings
from abc import abstractmethod
from typing import Any, overload

import pydantic
from pydantic import (
    BaseModel,
    ValidationInfo,
    computed_field,
    field_serializer,
    field_validator,
    model_serializer,
    model_validator,
)
from pydantic.functional_validators import field_validator as fv

log = logging.getLogger(__name__)


class Signup(BaseModel):
    email: str
    name: str
    phone: str
    tags: list[str]
    age: int
    nick: str | None = None
    code: str
    slug: str

    # The common bug: only the error branch raises; the happy path falls off.
    @field_validator("email")
    @classmethod
    def email_not_taken(cls, v: str) -> str:  # caught
        if v.endswith("@example.com"):
            raise ValueError("taken")

    # if/else where one branch neither returns nor raises.
    @field_validator("name", mode="before")
    @classmethod
    def strip_name(cls, v):  # caught
        if isinstance(v, str):
            return v.strip()
        else:
            log.warning("name is not a string: %r", v)

    # A loop that validates items and never returns.
    @field_validator("tags")
    @classmethod
    def tags_lowercase(cls, v: list[str]):  # caught
        for tag in v:
            if tag != tag.lower():
                raise ValueError("tags must be lowercase")

    # A handler that logs and falls through.
    @field_validator("age", mode="wrap")
    @classmethod
    def age_lenient(cls, v, handler):  # caught
        try:
            return handler(v)
        except ValueError:
            log.exception("bad age")

    # A bare return: the field becomes None.
    @field_validator("phone")
    @classmethod
    def phone_digits(cls, v: str) -> str:
        if not v:
            return  # caught
        return "".join(c for c in v if c.isdigit())

    # An elif chain with no else.
    @fv("code", mode="plain")
    @classmethod
    def code_upper(cls, v):  # caught
        if isinstance(v, str):
            return v.upper()
        elif isinstance(v, int):
            return str(v)

    # `assert` is not a way out.
    @pydantic.field_validator("slug")
    @classmethod
    def slug_ascii(cls, v: str, info: ValidationInfo):  # caught
        assert v.isascii(), "slug must be ASCII"


class Order(BaseModel):
    quantity: int
    price: float

    # An after model validator that raises on a condition and otherwise ends: no `return self`.
    @model_validator(mode="after")
    def check_total(self):  # caught
        if self.quantity * self.price > 10_000:
            raise ValueError("order too large")

    # A before model validator that mutates `data` in place and forgets `return data`.
    @model_validator(mode="before")
    @classmethod
    def fill_price(cls, data: Any):  # caught
        if isinstance(data, dict):
            data.setdefault("price", 0.0)

    # A wrap model validator with a bare return.
    @model_validator(mode="wrap")
    @classmethod
    def log_errors(cls, data, handler):
        try:
            return handler(data)
        except pydantic.ValidationError:
            log.info("invalid order")
            return  # caught


# ---- near misses ------------------------------------------------------------------------------------------------


class Profile(BaseModel):
    email: str
    name: str
    phone: str | None
    tags: list[str]
    age: int
    nick: str | None = None
    code: str
    city: str
    zip: str
    bio: str

    # Every path returns or raises.
    @field_validator("email")
    @classmethod
    def email_ok(cls, v: str) -> str:
        if v.endswith("@example.com"):
            raise ValueError("taken")
        return v

    @field_validator("name")
    @classmethod
    def name_ok(cls, v: str) -> str:
        if v:
            return v.strip()
        else:
            raise ValueError("required")

    # A `raise` in every handler.
    @field_validator("age", mode="wrap")
    @classmethod
    def age_ok(cls, v, handler):
        try:
            return handler(v)
        except ValueError as exc:
            log.exception("bad age")
            raise ValueError("age") from exc

    # `return None` written on purpose: the field is meant to become None.
    @field_validator("phone", mode="before")
    @classmethod
    def blank_phone(cls, v):
        if v == "":
            return None
        return v

    # A loop that returns, and a loop with an `else` that raises.
    @field_validator("tags")
    @classmethod
    def tags_ok(cls, v: list[str]) -> list[str]:
        for tag in v:
            if tag != tag.lower():
                raise ValueError("tags must be lowercase")
        return v

    @field_validator("code")
    @classmethod
    def code_ok(cls, v: str) -> str:
        for prefix in ("A", "B"):
            if v.startswith(prefix):
                break
        else:
            raise ValueError("bad prefix")
        return v

    # `while True` that only leaves by returning.
    @field_validator("city")
    @classmethod
    def city_ok(cls, v: str) -> str:
        while True:
            if v.strip() == v:
                return v
            v = v.strip()

    # A match whose every case returns, with a catch-all.
    @field_validator("zip")
    @classmethod
    def zip_ok(cls, v):
        match v:
            case int():
                return str(v)
            case _:
                return v

    # A nested function with a bare return is that function's, not the validator's.
    @field_validator("bio")
    @classmethod
    def bio_ok(cls, v: str) -> str:
        def first(lines):
            if not lines:
                return
            return lines[0]

        first(v.splitlines())
        return v

    # Not validators: serializers and computed fields return what they like.
    @field_serializer("email")
    def dump_email(self, v: str):
        if v:
            log.info("dumping")

    @model_serializer(mode="wrap")
    def dump(self, handler):
        handler(self)

    @computed_field
    @property
    def initials(self) -> str:
        if self.name:
            log.info("no initials")


class Shipment(BaseModel):
    weight: float
    note: str

    # `return self` behind a `with` block.
    @model_validator(mode="after")
    def check_weight(self):
        with warnings.catch_warnings():
            if self.weight < 0:
                raise ValueError("negative")
            return self

    # A before model validator that returns the data on every path.
    @model_validator(mode="before")
    @classmethod
    def defaults(cls, data):
        if isinstance(data, dict):
            data.setdefault("note", "")
        return data

    # A generator is not a validator Pydantic can use; its return is StopIteration's.
    @field_validator("note")
    @classmethod
    def note_parts(cls, v):
        for part in v.split():
            yield part


class BaseSchema(BaseModel):
    value: str

    # An abstract stub: a docstring and `pass`, or `...`.
    @field_validator("value")
    @classmethod
    @abstractmethod
    def check_value(cls, v):
        """Subclasses validate the value."""
        pass

    @model_validator(mode="after")
    @abstractmethod
    def check_model(self):
        ...

    # `raise NotImplementedError` is a raise.
    @model_validator(mode="before")
    @classmethod
    def prepare(cls, data):
        raise NotImplementedError

    # An overload stub.
    @field_validator("value", mode="plain")
    @classmethod
    @overload
    def coerce(cls, v: int) -> str: ...
