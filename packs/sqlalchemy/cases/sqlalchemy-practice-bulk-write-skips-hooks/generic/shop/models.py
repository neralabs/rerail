from sqlalchemy import Column, Integer, String, event
from sqlalchemy.orm import DeclarativeBase, Session


class Base(DeclarativeBase):
    pass


class Order(Base):
    __tablename__ = "order"
    id = Column(Integer, primary_key=True)
    status = Column(String)


@event.listens_for(Order, "after_update")
@event.listens_for(Order, "after_delete")
def audit_order(mapper, connection, target):
    connection.info.setdefault("audit", []).append(target.id)


@event.listens_for(Session, "do_orm_execute")
def audit_bulk_deletes(state):
    if state.is_delete:
        state.session.info.setdefault("audit", []).append(state.statement)
