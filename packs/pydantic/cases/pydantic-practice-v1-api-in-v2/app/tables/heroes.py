from typing import Optional

import sqlmodel
from sqlmodel import Field, SQLModel
from sqlmodel import SQLModel as Table


class HeroBase(SQLModel):
    name: str


class Hero(HeroBase, table=True):
    id: Optional[int] = Field(default=None, primary_key=True)


class HeroRead(HeroBase):
    id: int


class HeroPublic(Table):
    name: str


class HeroModule(sqlmodel.SQLModel):
    name: str
