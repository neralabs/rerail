# Tests build throwaway models, and some deliberately leave a validator without a return to check Pydantic's
# behaviour: the rule leaves test code alone.
from pydantic import BaseModel, field_validator


def test_missing_return_gives_none():
    class M(BaseModel):
        x: int

        @field_validator("x")
        @classmethod
        def check(cls, v):
            if v < 0:
                raise ValueError("negative")

    assert M(x=1).x is None
