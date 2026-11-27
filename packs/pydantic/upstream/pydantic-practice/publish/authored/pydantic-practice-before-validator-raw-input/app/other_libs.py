import attrs
from pydantic.v1 import BaseModel as V1BaseModel
from pydantic.v1 import validator as v1_validator


def validator(*fields, pre=False):
    def wrap(fn):
        return fn

    return wrap


# A project's own `validator` decorator: not Pydantic's.
class Form:
    @validator("name", pre=True)
    def strip_name(cls, v):
        return v.strip()


# attrs validators get the attribute, not raw input.
@attrs.define
class Point:
    x: str = attrs.field()

    @x.validator
    def _check_x(self, attribute, value):
        value.strip()


# The pydantic.v1 compatibility layer: not read.
class OldUser(V1BaseModel):
    name: str

    @v1_validator("name", pre=True)
    def strip_name(cls, v):
        return v.strip()
