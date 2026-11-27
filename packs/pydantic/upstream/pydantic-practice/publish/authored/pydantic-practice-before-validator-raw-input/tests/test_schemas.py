import pytest
from pydantic import BaseModel, field_validator

from app.vendors import Draft


class Throwaway(BaseModel):
    x: str

    @field_validator("x", mode="before")
    @classmethod
    def strip(cls, v):
        return v.strip()


def test_raw_input_raises():
    with pytest.raises(AttributeError):
        Throwaway.model_validate({"x": 1})


def test_draft():
    assert Draft.model_validate({"title": " a "}).title == "a"
