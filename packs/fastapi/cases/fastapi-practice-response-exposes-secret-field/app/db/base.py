from sqlalchemy import Column, String
from sqlalchemy.orm import DeclarativeBase, Mapped, declarative_base, mapped_column


class Base(DeclarativeBase):
    pass


LegacyBase = declarative_base()


class CredentialsMixin:
    hashed_password: Mapped[str] = mapped_column(String(255))


class KeyMixin:
    api_key = Column(String(64))
