from pydantic import BaseModel, validator


# Pydantic 1 turns TypeError (subscripts, len, unpacking) into a validation error, but not AttributeError.
class Account(BaseModel):
    name: str
    ref: str = ""

    @validator("name", pre=True)
    def strip_name(cls, v):
        return v.strip()  # caught

    @validator("ref", pre=True)
    def ref_from_mapping(cls, v):
        return v["id"]


def load_account(raw: str) -> Account:
    return Account.parse_raw(raw)
