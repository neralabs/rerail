from pydantic import BaseModel, Field

from app.schemas import User


class Probe(BaseModel):
    code: str = Field(..., regex=r"^\d+$")  # caught


def test_user_roundtrip():
    user = User.parse_obj({"id": 1, "email": "a@b.c"})
    assert user.dict()["id"] == 1
    assert user.json(sort_keys=True)  # caught
