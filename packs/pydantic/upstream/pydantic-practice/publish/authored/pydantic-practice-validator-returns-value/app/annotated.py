import sys
from typing import Annotated, NoReturn, Optional

from pydantic import AfterValidator, BaseModel, BeforeValidator, field_validator, model_validator

from app import errors
from app.errors import Errors, fail_soft, never
from app.errors import reject_twice as refuse


def fail(msg: str):
    raise ValueError(msg)


def reject(msg: str) -> NoReturn:
    log_rejection(msg)
    raise ValueError(msg)


def log_rejection(msg: str) -> None:
    print(msg)


# A function attached through Annotated: its return value becomes the field's value.
def must_be_upper(v: str) -> str:  # caught
    if v != v.upper():
        raise ValueError("upper")


def strip_or_none(v):
    if v == "":
        return  # caught
    return v.strip()


def must_be_ascii(v: str) -> str:
    if not v.isascii():
        raise ValueError("ascii")
    return v


Upper = Annotated[str, AfterValidator(must_be_upper)]


class Code(BaseModel):
    code: Upper
    label: Annotated[str, BeforeValidator(strip_or_none)] = ""
    ascii_name: Annotated[str, AfterValidator(must_be_ascii)] = ""


class Contact(BaseModel):
    email: Optional[str] = None
    phone: Optional[str] = None
    city: str = ""

    # `return None` written out: None is meant.
    @field_validator("email")
    @classmethod
    def empty_to_none(cls, v):
        if v == "":
            return None
        return v

    # The last statement calls a helper that always raises.
    @field_validator("email")
    @classmethod
    def check(cls, v):
        if v and "@" in v:
            return v
        fail("bad email")

    @field_validator("phone")
    @classmethod
    def check_phone(cls, v):
        if v is None or v.isdigit():
            return v
        reject("bad phone")

    @field_validator("city")
    @classmethod
    def check_city(cls, v):
        if v:
            return v
        sys.exit("no city")

    # A helper that only logs does return.
    @field_validator("city")
    @classmethod
    def log_city(cls, v):  # caught
        if v:
            return v
        log_rejection("empty city")


class Order(BaseModel):
    qty: int

    # An after model validator's None is what `model_validate()` returns.
    @model_validator(mode="after")
    def positive(self):
        if self.qty < 0:
            raise ValueError("neg")
        return None  # caught

    @model_validator(mode="after")
    def capped(self):
        if self.qty > 100:
            return  # caught
        return self


# Helpers in another module: imported under an alias, through the module, a staticmethod, a Never annotation.
class Account(BaseModel):
    a: str = ""
    b: str = ""
    c: str = ""
    d: str = ""
    e: str = ""

    @field_validator("a")
    @classmethod
    def check_a(cls, v):
        if v:
            return v
        refuse("a")

    @field_validator("b")
    @classmethod
    def check_b(cls, v):
        if v:
            return v
        errors.bail("b")

    @field_validator("c")
    @classmethod
    def check_c(cls, v):
        if v:
            return v
        Errors.invalid("c")

    @field_validator("d")
    @classmethod
    def check_d(cls, v):
        if v:
            return v
        never("d")

    # Named like a raising helper, but it returns: the field becomes None.
    @field_validator("e")
    @classmethod
    def check_e(cls, v):  # caught
        if v:
            return v
        fail_soft("e")
