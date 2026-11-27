from pydantic import BaseModel, Field, validator

from app.schemas import Account, User


class Probe(BaseModel):
    codes: list[str] = Field(default_factory=list, min_items=1)

    class Config:
        extra = "forbid"

    @validator("codes")
    def strip(cls, v):
        return v


def test_user_roundtrip(acct: Account):
    user = User.parse_obj({"id": 1, "email": "a@b.c"})
    assert user.dict()["id"] == 1
    assert acct.json()
    assert list(Account.__fields__)
