"""Imported by nothing: a model of the package whose module the ledger's metadata imports."""
from sqlalchemy import Column, Integer, Text

from shared_models.entries import Base


class AuditEntry(Base):
    __tablename__ = "audit"
    id = Column(Integer, primary_key=True)
    actor = Column(Text)
