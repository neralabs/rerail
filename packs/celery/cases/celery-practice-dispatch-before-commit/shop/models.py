from dataclasses import dataclass
from datetime import datetime

from sqlalchemy import Column, Integer, String
from sqlalchemy.orm import DeclarativeBase, Mapped, declarative_base, mapped_column

LegacyBase = declarative_base()


class Base(DeclarativeBase):
    pass


class Audited:
    updated_at: Mapped[datetime] = mapped_column(nullable=True)


class Order(Audited, Base):
    __tablename__ = "orders"
    id: Mapped[int] = mapped_column(primary_key=True)
    paid: Mapped[bool] = mapped_column(default=False)
    status: Mapped[str] = mapped_column(default="new")

    def __init__(self, **kw):
        super().__init__(**kw)
        self.note = ""


class Invoice(LegacyBase):
    __tablename__ = "invoices"
    id = Column(Integer, primary_key=True)
    state = Column(String, default="open")
    label = "invoice"


@dataclass
class OrderView:
    id: int
    status: str
