from typing import Optional

from pydantic import BaseModel, BaseSettings, Field, root_validator, validator


class Settings(BaseSettings):
    database_url: str = "sqlite://"


class User(BaseModel):
    id: int
    email: str = Field(..., regex=r"^\S+@\S+$")
    display: Optional[str] = None

    class Config:
        orm_mode = True
        allow_mutation = False

    @validator("email")
    def lower_email(cls, v):
        return v.lower()

    @root_validator
    def check_display(cls, values):
        return values


def load(data: dict, row):
    user = User.parse_obj(data)
    other = User.from_orm(row)
    return user.dict(), other.json(sort_keys=True), User.schema()
