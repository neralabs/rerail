from sqlalchemy import Boolean, Column
from sqlalchemy.orm import DeclarativeBase


class Base(DeclarativeBase):
    pass


class SoftDeleteMixin:
    is_deleted = Column(Boolean, default=False)


class TokenMixin:
    legacy_token = Column(Boolean)
    api_token = Column(Boolean)
