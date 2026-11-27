from sqlalchemy import Column, ForeignKey, Integer, String
from sqlalchemy.orm import DeclarativeBase, Mapped, mapped_column, relationship


class Base(DeclarativeBase):
    pass


class User(Base):
    __tablename__ = "users"
    id: Mapped[int] = mapped_column(primary_key=True)
    name: Mapped[str] = mapped_column(String(50))
    orders: Mapped[list["Order"]] = relationship(back_populates="customer")


class Order(Base):
    __tablename__ = "orders"
    id: Mapped[int] = mapped_column(primary_key=True)
    total: Mapped[int]
    customer_id: Mapped[int] = mapped_column(ForeignKey("users.id"))
    customer: Mapped["User"] = relationship(back_populates="orders")
    lines: Mapped[list["Line"]] = relationship(back_populates="order")
    coupon: Mapped["Coupon"] = relationship(lazy="joined")


class Coupon(Base):
    __tablename__ = "coupons"
    id: Mapped[int] = mapped_column(primary_key=True)
    order_id: Mapped[int] = mapped_column(ForeignKey("orders.id"))


class Line(Base):
    __tablename__ = "lines"
    id: Mapped[int] = mapped_column(primary_key=True)
    qty: Mapped[int]
    order_id: Mapped[int] = mapped_column(ForeignKey("orders.id"))
    order: Mapped[Order] = relationship(back_populates="lines")
    # no key on either table: the direction is unknown, so this rule takes it
    product = relationship("Product")


class Product(Base):
    __tablename__ = "products"
    id: Mapped[int] = mapped_column(primary_key=True)


# Legacy declarations with no annotation: the direction comes from the foreign keys.
class Warehouse(Base):
    __tablename__ = "warehouses"
    id = Column(Integer, primary_key=True)
    region_id = Column(Integer, ForeignKey("regions.id"))
    manager_id = Column(Integer, ForeignKey("staff.id"))
    region = relationship("Region")
    manager = relationship("Staff", foreign_keys=[manager_id])
    staff = relationship("Staff", foreign_keys="Staff.warehouse_id")
    keeper = relationship("Keeper", uselist=False)


class Region(Base):
    __tablename__ = "regions"
    id = Column(Integer, primary_key=True)
    # the key is on Invoice: one-to-many, and its backref gives Invoice a single `region`
    invoices = relationship("Invoice", backref="region")


class Invoice(Base):
    __tablename__ = "invoices"
    id = Column(Integer, primary_key=True)
    region_id = Column(Integer, ForeignKey("regions.id"))


class Staff(Base):
    __tablename__ = "staff"
    id = Column(Integer, primary_key=True)
    warehouse_id = Column(Integer, ForeignKey("warehouses.id"))


class Keeper(Base):
    __tablename__ = "keepers"
    id = Column(Integer, primary_key=True)
    warehouse_id = Column(Integer, ForeignKey("warehouses.id"))
