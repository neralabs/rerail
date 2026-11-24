from datetime import datetime

from pydantic import SecretStr
from sqlalchemy import Column, Integer, String
from sqlalchemy.orm import Mapped, mapped_column
from sqlmodel import Field, SQLModel

from app.db.base import Base, CredentialsMixin, KeyMixin, LegacyBase


class User(Base, CredentialsMixin):
    __tablename__ = "user"

    id: Mapped[int] = mapped_column(primary_key=True)
    email: Mapped[str] = mapped_column(String(255))
    password_changed_at: Mapped[datetime] = mapped_column()
    password_reset_required: Mapped[bool] = mapped_column(default=False)
    has_password: Mapped[bool] = mapped_column(default=True)


class Service(LegacyBase, KeyMixin):
    __tablename__ = "service"

    id = Column(Integer, primary_key=True)
    name = Column(String(64))


class OAuthAccount(SQLModel, table=True):
    id: int | None = Field(default=None, primary_key=True)
    access_token: str
    refresh_token: str | None = None


class ApiToken(Base):
    __tablename__ = "api_token"

    id: Mapped[int] = mapped_column(primary_key=True)
    refresh_token: Mapped[str | None] = mapped_column(String(1024))


class MemberBase(SQLModel):
    email: str


class MemberSecrets(SQLModel):
    password_hash: str


class Member(MemberBase, MemberSecrets, table=True):
    id: int | None = Field(default=None, primary_key=True)


class MfaDevice(Base):
    __tablename__ = "mfa_device"

    id: Mapped[int] = mapped_column(primary_key=True)
    totp_secret = Column(String(64))


class Hero(SQLModel, table=True):
    id: int | None = Field(default=None, primary_key=True)
    name: str
    secret_name: str


class Integration(SQLModel, table=True):
    id: int | None = Field(default=None, primary_key=True)
    client_id: str
    client_secret: str


class Device(SQLModel, table=True):
    id: int | None = Field(default=None, primary_key=True)
    name: str
    private_key: str


class Agent(SQLModel, table=True):
    id: int | None = Field(default=None, primary_key=True)
    name: str
    api_key: str = Field(exclude=True)


class Webhook(SQLModel, table=True):
    id: int | None = Field(default=None, primary_key=True)
    url: str
    secret: SecretStr
