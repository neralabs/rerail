from typing import Callable, Optional

from pydantic import BaseModel, ConfigDict, Field, field_validator, model_validator, root_validator, validator

from app.base import OrmBase, PlainBase
from app.checks import validator as check


class Tag(OrmBase):
    name: str = Field(..., regex=r"^[a-z]+$")  # caught
    kind: str = Field("tag", const=True)  # caught
    aliases: list[str] = Field(default_factory=list, unique_items=True)  # caught
    labels: list[str] = Field(default_factory=list, min_items=1, max_items=5)  # caught
    slug: str = Field(..., pattern=r"^[a-z-]+$", min_length=1)


class User(PlainBase):
    id: int
    email: str
    display: Optional[str] = None

    class Config:  # caught
        orm_mode = True  # caught
        allow_population_by_field_name = True  # caught
        fields = {"email": {"alias": "mail"}}  # caught
        allow_mutation = False  # caught
        extra = "ignore"

    @validator("email")  # caught
    def lower_email(cls, v):
        return v.lower()

    @root_validator  # caught
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


class Rule(BaseModel):
    name: str
    validate: Callable[[str], bool]
