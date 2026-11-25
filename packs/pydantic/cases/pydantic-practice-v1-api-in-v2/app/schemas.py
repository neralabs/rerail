from typing import Callable, Optional

import pydantic as pd
from pydantic import BaseModel, ConfigDict, Field, field_validator, model_validator, root_validator, validator
from pydantic import root_validator as post_check
from sqlmodel import Field as SQLField
from sqlmodel import SQLModel

from app.base import OrmBase, PlainBase
from app.checks import validator as check


class Tag(OrmBase):
    name: str = Field(..., regex=r"^[a-z]+$")  # caught
    kind: str = Field("tag", const=True)  # caught
    aliases: list[str] = Field(default_factory=list, unique_items=True)  # caught
    labels: list[str] = Field(default_factory=list, min_items=1, max_items=5)
    slug: str = Field(..., pattern=r"^[a-z-]+$", min_length=1)


class Code(pd.BaseModel):
    value: str = pd.Field(..., regex=r"^[0-9]+$")  # caught


class TagRow(SQLModel):
    name: str = SQLField(regex=r"^[a-z]+$")


class User(PlainBase):
    id: int
    email: str
    display: Optional[str] = None

    class Config:
        orm_mode = True  # caught
        allow_population_by_field_name = True  # caught
        fields = {"email": {"alias": "mail"}}  # caught
        allow_mutation = False  # caught
        extra = "ignore"

    @validator("email")
    def lower_email(cls, v):
        return v.lower()

    @root_validator  # caught
    def check_display(cls, values):
        return values

    @root_validator(pre=True)
    def fill_display(cls, values):
        return values

    @root_validator(skip_on_failure=True)
    def check_more(cls, values):
        return values

    @post_check()  # caught
    def check_last(cls, values):
        return values


class Account(BaseModel):
    model_config = ConfigDict(from_attributes=True, str_strip_whitespace=True)

    id: int
    owner: User

    @field_validator("id")
    @classmethod
    def positive(cls, v: int) -> int:
        return v

    @model_validator(mode="after")
    def check(self) -> "Account":
        return self

    @check("id")
    def local_check(cls, v):
        return v


class Report(BaseModel):
    title: str

    def json(self, **kwargs):
        return self.model_dump_json(indent=kwargs.get("indent"))


class Rule(BaseModel):
    name: str
    validate: Callable[[str], bool]
