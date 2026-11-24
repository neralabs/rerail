from typing import Annotated, Generic, Optional, TypeVar

from pydantic import BaseModel, Field, SecretStr

T = TypeVar("T")


class UserBase(BaseModel):
    email: str


class UserPublic(UserBase):
    id: int


class UserCreate(UserBase):
    password: str


class UserInDB(UserPublic):
    hashed_password: str


class UserWithFlags(UserPublic):
    password_changed_at: str | None = None
    has_password: bool = True


class Token(BaseModel):
    access_token: str
    refresh_token: str
    token_type: str = "bearer"


class SmtpSettings(BaseModel):
    host: str
    password: Optional[SecretStr] = None


class ServiceOut(BaseModel):
    name: str
    api_key: str = Field(exclude=True)


class ServiceWithKey(BaseModel):
    name: str
    api_key: Annotated[str, Field(min_length=8)]


class Page(BaseModel, Generic[T]):
    items: list[T]
    total: int


UserList = list[UserInDB]
