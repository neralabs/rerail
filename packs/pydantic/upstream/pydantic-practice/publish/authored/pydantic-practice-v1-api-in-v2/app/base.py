from pydantic import BaseModel, ConfigDict


class OrmBase(BaseModel):
    model_config = ConfigDict(from_attributes=True, populate_by_name=True)


class PlainBase(BaseModel):
    model_config = ConfigDict(extra="forbid")
