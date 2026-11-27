from sqlalchemy import Column, Integer
from sqlalchemy.orm import declarative_base

LegacyBase = declarative_base()


class Invoice(LegacyBase):
    __tablename__ = "invoices"
    id = Column(Integer, primary_key=True)
    total = Column(Integer)
