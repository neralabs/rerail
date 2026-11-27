from sqlalchemy import Column, Integer, Numeric, Text
from sqlalchemy.orm import declarative_base

Base = declarative_base()


class Entry(Base):
    __tablename__ = "entries"
    id = Column(Integer, primary_key=True)
    memo = Column(Text)
    amount = Column(Numeric)
