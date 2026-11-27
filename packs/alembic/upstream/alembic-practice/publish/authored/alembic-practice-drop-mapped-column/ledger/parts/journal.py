from sqlalchemy import Column, Integer, String

from shared_models.entries import Base

__all__ = ["Journal"]


class Journal(Base):
    __tablename__ = "journal"
    id = Column(Integer, primary_key=True)
    ref = Column(String(40))
