from datetime import datetime, timezone

from sqlalchemy import Boolean, Column, DateTime, ForeignKey, Integer, String, Text, event, update
from sqlalchemy.orm import validates

from app.db import Base, db


class Stamped:
    updated_at = Column(DateTime)

    @staticmethod
    def _touch(mapper, connection, target):
        target.updated_at = datetime.now(timezone.utc)

    @classmethod
    def __declare_last__(cls):
        event.listen(cls, "before_update", cls._touch)


class Item(Base):
    __tablename__ = "item"
    id = Column(Integer, primary_key=True)
    list_id = Column(Integer)
    label = Column(String)


class Tag(Base):
    __tablename__ = "tag"
    id = Column(Integer, primary_key=True)
    name = Column(String)
    color = Column(String)

    @validates("name")
    def _normalize(self, key, value):
        return value.strip().lower()


class Unit(Base):
    __tablename__ = "unit"
    id = Column(Integer, primary_key=True)
    name = Column(String)
    normalized_name = Column(String)
    abbreviation = Column(String)


class Note(Base):
    __tablename__ = "note"
    id = Column(Integer, primary_key=True)
    title = Column(String)


class Ticket(Stamped, Base):
    __tablename__ = "ticket"
    id = Column(Integer, primary_key=True)
    closed = Column(Boolean)


class Audited(Base):
    __abstract__ = True
    id = Column(Integer, primary_key=True)


class Invoice(Audited):
    __tablename__ = "invoice"
    total = Column(Integer)


class Report(Base):
    __tablename__ = "report"
    id = Column(Integer, primary_key=True)
    kind = Column(String)
    __mapper_args__ = {"polymorphic_on": kind, "polymorphic_identity": "report"}


class MonthlyReport(Report):
    __tablename__ = "monthly_report"
    id = Column(Integer, ForeignKey("report.id"), primary_key=True)
    __mapper_args__ = {"polymorphic_identity": "monthly"}


class AuditedDoc(Base):
    __tablename__ = "audited_doc"
    id = Column(Integer, primary_key=True)
    body = Column(Text)


class Dataset(Base):
    __tablename__ = "dataset"
    id = Column(Integer, primary_key=True)
    name = Column(String)
    version = Column(Integer)

    @staticmethod
    def before_update(mapper, connection, target):
        connection.execute(update(Dataset).where(Dataset.id == target.id).values(version=Dataset.version + 1))


event.listen(Dataset, "before_update", Dataset.before_update)


class Account(db.Model):
    id = db.Column(db.Integer, primary_key=True)
    active = db.Column(db.Boolean)
    version = db.Column(db.Integer)
