from pydantic.v1 import BaseModel, Field, root_validator, validator


class Legacy(BaseModel):
    name: str = Field(..., regex=r"^[a-z]+$")

    class Config:
        orm_mode = True
        allow_mutation = False

    @validator("name")
    def lower(cls, v):
        return v.lower()

    @root_validator
    def check(cls, values):
        return values


def use(row, data: dict):
    item = Legacy.from_orm(row)
    other = Legacy.parse_obj(data)
    return item.dict(), other.json(sort_keys=True)
