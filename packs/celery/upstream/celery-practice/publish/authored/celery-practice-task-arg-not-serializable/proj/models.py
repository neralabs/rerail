from sqlalchemy.orm import DeclarativeBase, Mapped, mapped_column


class Base(DeclarativeBase):
    pass


class Order(Base):
    __tablename__ = "orders"
    id: Mapped[int] = mapped_column(primary_key=True)

    def notify(self):
        from proj.tasks import ship

        ship.delay(self)  # caught
        ship.delay(self.id)


class OrderIn:
    def __init__(self, total):
        self.total = total
