from sqlalchemy import Column, DateTime, Integer
from sqlalchemy.orm import DeclarativeBase, declared_attr


class Base(DeclarativeBase):
    # Table names come from the class name (Netflix/dispatch's way): no model declares __tablename__.
    @declared_attr.directive
    def __tablename__(cls):
        return cls.__name__.lower()


class TimestampMixin:
    created_at = Column(DateTime)


class AuditedBase(Base):
    # An abstract base: no table of its own.
    __abstract__ = True
    revision = Column(Integer)
