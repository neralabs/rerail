from sqlalchemy import select
from sqlalchemy.orm import Session, joinedload

from app.models import Line, Order

ORDER_OPTIONS = [joinedload(Order.customer)]


def get_all(*, db_session: Session):
    return db_session.query(Order).all()


def lines_of(session: Session, order_id: int):
    return session.scalars(select(Line).where(Line.order_id == order_id)).all()


def big_lines(session: Session):
    return session.scalars(select(Line).where(Line.qty > 10)).all()
