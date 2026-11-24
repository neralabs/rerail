from sqlalchemy import Column, Integer

from app.models.base import Base


class Widget(Base):
    __tablename__ = "widgets"
    id = Column(Integer, primary_key=True)
    size = Column(Integer)
