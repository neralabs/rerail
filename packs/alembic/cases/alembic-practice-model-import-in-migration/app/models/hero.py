from sqlmodel import Field, SQLModel


class HeroBase(SQLModel):
    name: str = Field(max_length=255)


class Hero(HeroBase, table=True):
    id: int | None = Field(default=None, primary_key=True)
    secret_name: str


class HeroPublic(HeroBase):
    id: int
