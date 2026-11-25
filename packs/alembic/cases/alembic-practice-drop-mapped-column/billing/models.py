"""Another service of the monorepo, with its own database and its own Alembic tree (billing/migrations)."""
from sqlalchemy import Column, Integer, String, Text
from sqlalchemy.orm import declarative_base

Base = declarative_base()


class Customer(Base):
    __tablename__ = "users"
    id = Column(Integer, primary_key=True)
    nick = Column(String(40))
    wallet_id = Column(Integer)


class Invoice(Base):
    __tablename__ = "invoices"
    id = Column(Integer, primary_key=True)
    memo = Column(Text)
