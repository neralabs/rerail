# app/plugins/ has no __init__.py (a namespace package, as Netflix/dispatch's dispatch/feedback/).
from sqlalchemy import Column, Integer

from app.models.base import Base


class Payment(Base):
    id = Column(Integer, primary_key=True)
    amount = Column(Integer)
