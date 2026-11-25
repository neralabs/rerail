from typing import Callable, Optional

import pydantic as pd
from pydantic import BaseModel, ConfigDict, Field, field_validator, model_validator, root_validator, validator
from pydantic import validator as field_check
from sqlmodel import Field as SQLField
from sqlmodel import SQLModel

from app.base import OrmBase, PlainBase
from app.checks import validator as check


class Tag(OrmBase):
    name: str = Field(..., regex=r"^[a-z]+$")
    labels: list[str] = Field(default_factory=list, min_items=1, max_items=5)  # caught
    codes: list[str] = Field(default_factory=list, max_items=3, unique_items=True)
    slug: str = Field(..., pattern=r"^[a-z-]+$", min_length=1)


class TagRow(SQLModel):
    names: list[str] = SQLField(default_factory=list, min_items=1)


class User(PlainBase):
    id: int
    email: str
    display: Optional[str] = None

    class Config:  # caught
        orm_mode = True
        allow_population_by_field_name = True
        extra = "ignore"

    @validator("email")  # caught
    def lower_email(cls, v):
        return v.lower()

    @field_check("display")  # caught
    def strip_display(cls, v):
        return v

    @pd.validator("id")  # caught
    def positive_id(cls, v):
        return v

    @root_validator
    def check_display(cls, values):
        return values

    @root_validator(pre=True)  # caught
    def fill_display(cls, values):
        return values

    @root_validator(skip_on_failure=True)  # caught
    def check_more(cls, values):
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

    def dict(self, **kwargs):
        data = super().model_dump(**kwargs)
        data["title"] = data["title"].upper()
        return data


class TitledReport(Report):
    subtitle: str = ""


class Rule(BaseModel):
    name: str
    validate: Callable[[str], bool]
