from pydantic import BaseModel, ConfigDict

from app.tables import Hero, HeroModule, HeroPublic, HeroRead


class UserRead(BaseModel):
    model_config = ConfigDict(orm_mode=True)
    name: str


class UserKeyword(BaseModel, from_attributes=True):
    name: str


class UserNested(BaseModel):
    name: str

    class Config:
        from_attributes = True


class UserDict(BaseModel):
    model_config = {"from_attributes": True}
    name: str


class UserOff(UserKeyword):
    model_config = ConfigDict(from_attributes=False)


class UserOffChild(UserOff):
    nickname: str = ""


class UserBackOn(UserOff):
    model_config = ConfigDict(from_attributes=True)


def read_hero(row: Hero) -> HeroRead:
    return HeroRead.from_orm(row)


def read_others(row: Hero):
    public = HeroPublic.from_orm(row)
    module = HeroModule.from_orm(row)
    keyword = UserKeyword.from_orm(row)
    nested = UserNested.from_orm(row)
    literal = UserDict.from_orm(row)
    back_on = UserBackOn.from_orm(row)
    return public, module, keyword, nested, literal, back_on


def read_user(row) -> dict:
    return UserRead.from_orm(row).model_dump()  # caught


def read_off(row):
    off = UserOff.from_orm(row)  # caught
    child = UserOffChild.from_orm(row)  # caught
    return off, child
