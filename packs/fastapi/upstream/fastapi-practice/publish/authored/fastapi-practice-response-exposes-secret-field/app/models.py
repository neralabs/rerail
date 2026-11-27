from datetime import datetime

from pydantic import SecretStr
from sqlalchemy import Boolean, Column, Integer, String
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


# Other spellings of a password hash, and stored secrets and keys by suffix: a row sent as it is.
class Login(Base):
    __tablename__ = "login"

    id: Mapped[int] = mapped_column(primary_key=True)
    password_digest: Mapped[str] = mapped_column(String(255))


class LegacyLogin(Base):
    __tablename__ = "legacy_login"

    id: Mapped[int] = mapped_column(primary_key=True)
    hashed_pw: Mapped[str] = mapped_column(String(255))


class Hook(Base):
    __tablename__ = "hook"

    id: Mapped[int] = mapped_column(primary_key=True)
    webhook_secret: Mapped[str] = mapped_column(String(64))
    is_secret: Mapped[bool] = mapped_column(default=False)


class Provider(Base):
    __tablename__ = "provider"

    id: Mapped[int] = mapped_column(primary_key=True)
    openai_api_key: Mapped[str] = mapped_column(String(128))


# Flags, not values: left alone.
class Flagged(Base):
    __tablename__ = "flagged"

    id: Mapped[int] = mapped_column(primary_key=True)
    is_secret: Mapped[bool] = mapped_column(default=False)
    has_api_key: Mapped[bool] = mapped_column(default=False)


# Words, not a list: a prefixed hash, a stored plain password, and flags typed bool.
class Mailer(Base):
    __tablename__ = "mailer"

    id: Mapped[int] = mapped_column(primary_key=True)
    smtp_password: Mapped[str] = mapped_column(String(255))


class Operator(Base):
    __tablename__ = "operator"

    id: Mapped[int] = mapped_column(primary_key=True)
    user_pwd_hash = Column(String(255))


class Toggles(Base):
    __tablename__ = "toggles"

    id: Mapped[int] = mapped_column(primary_key=True)
    has_password: Mapped[bool] = mapped_column(default=False)
    uses_api_key = Column(Boolean, default=False)
    show_secret = Column(Boolean)
