from typing import Annotated, Generic, Optional, TypeVar

from pydantic import BaseModel, Field, PlainSerializer, SecretStr, field_serializer, model_serializer
from pydantic import field_serializer as fs

from app.audit import field_serializer as audit_serializer

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


# A schema built to show a device password to its owner: reported, and the message says to leave it.
class WifiOut(BaseModel):
    ssid: str
    password: str


# The field serializer decides how the password is sent.
class MaskedOut(BaseModel):
    email: str
    password: str

    @field_serializer("password")
    def mask(self, value: str) -> str:
        return "***"


class MaskedChild(MaskedOut):
    name: str


# A model serializer decides every field.
class Exported(BaseModel):
    email: str
    hashed_password: str

    @model_serializer
    def dump(self) -> dict:
        return {"email": self.email}


# A key suffix on a schema, not a table: a design choice.
class HookCreated(BaseModel):
    id: int
    webhook_secret: str


# The serializer decorators under an alias, and a PlainSerializer in the field's metadata: still pydantic's.
class AliasMasked(BaseModel):
    password: str

    @fs("password")
    def mask(self, value: str) -> str:
        return "***"


class AnnotatedMasked(BaseModel):
    password: Annotated[str, PlainSerializer(lambda v: "***")]


# Another library's look-alike decorator: pydantic still sends the password.
class LookAlike(BaseModel):
    password: str

    @audit_serializer("password")
    def log(self, value: str) -> str:
        return value
