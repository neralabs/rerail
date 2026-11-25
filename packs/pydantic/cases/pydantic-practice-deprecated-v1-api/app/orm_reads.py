from pydantic import BaseModel, ConfigDict

from app.tables import Hero, HeroModule, HeroPublic, HeroRead


class UserRead(BaseModel):
    model_config = ConfigDict(orm_mode=True)
    name: str


class UserKeyword(BaseModel, from_attributes=True):
    name: str


class UserOff(UserKeyword):
    model_config = ConfigDict(from_attributes=False)


def read_hero(row: Hero) -> HeroRead:
    return HeroRead.from_orm(row)  # caught


def read_others(row: Hero):
    public = HeroPublic.from_orm(row)  # caught
    module = HeroModule.from_orm(row)  # caught
    keyword = UserKeyword.from_orm(row)  # caught
    dumped = public.dict()  # caught
    return public, module, keyword, dumped


def read_crashes(row):
    user = UserRead.from_orm(row)
    off = UserOff.from_orm(row)
    return user, off
