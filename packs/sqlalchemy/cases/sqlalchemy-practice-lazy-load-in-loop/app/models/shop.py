from sqlalchemy import ForeignKey, String
from sqlalchemy.orm import Mapped, mapped_column, relationship

from app.models.base import Base


class User(Base):
    __tablename__ = "users"
    id: Mapped[int] = mapped_column(primary_key=True)
    name: Mapped[str] = mapped_column(String(50))
    # lazy="select", the default: one SELECT per user on first read
    orders: Mapped[list["Order"]] = relationship(back_populates="customer")
    # loaded with the user
    profile: Mapped["Profile"] = relationship(lazy="joined")
    # never loads implicitly
    audit: Mapped[list["Audit"]] = relationship(lazy="raise")


class Profile(Base):
    __tablename__ = "profiles"
    id: Mapped[int] = mapped_column(primary_key=True)
    user_id: Mapped[int] = mapped_column(ForeignKey("users.id"))


class Audit(Base):
    __tablename__ = "audits"
    id: Mapped[int] = mapped_column(primary_key=True)
    user_id: Mapped[int] = mapped_column(ForeignKey("users.id"))


class Tag(Base):
    __tablename__ = "tags"
    id: Mapped[int] = mapped_column(primary_key=True)
    label: Mapped[str] = mapped_column(String(20))


class Order(Base):
    __tablename__ = "orders"
    id: Mapped[int] = mapped_column(primary_key=True)
    total: Mapped[int]
    customer_id: Mapped[int] = mapped_column(ForeignKey("users.id"))
    customer: Mapped["User"] = relationship(back_populates="orders")
    lines: Mapped[list["Line"]] = relationship(lazy="selectin")
    # gives Tag a lazy `orders` collection
    tags = relationship(Tag, secondary="order_tags", backref="orders")

    @classmethod
    def open_orders(cls, session) -> list["Order"]:
        qry = session.query(cls)
        qry = qry.filter(cls.total > 0)
        return qry.all()


class Line(Base):
    __tablename__ = "lines"
    id: Mapped[int] = mapped_column(primary_key=True)
    order_id: Mapped[int] = mapped_column(ForeignKey("orders.id"))
    product = relationship("Product")


class Product(Base):
    __tablename__ = "products"
    id: Mapped[int] = mapped_column(primary_key=True)


class GiftOrder(Order):
    # inherits Order.customer and Order.tags
    message: Mapped[str] = mapped_column(String(200))
