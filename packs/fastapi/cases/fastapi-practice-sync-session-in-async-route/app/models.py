from typing import ClassVar

from sqlalchemy.orm import DeclarativeBase

from .db import SessionType


class Base(DeclarativeBase):
    pass


class User(Base):
    __tablename__ = "users"


class Trade(Base):
    __tablename__ = "trades"
    session: ClassVar[SessionType]

    @staticmethod
    def rollback():
        Trade.session.rollback()

    @classmethod
    def open_count(cls):
        return cls.session.query(cls).count()

    @staticmethod
    def label():
        return "trade"
