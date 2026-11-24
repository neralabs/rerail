from sqlalchemy import select
from sqlalchemy.orm import Session, joinedload

from app.models.shop import Order


def get_all(*, db_session: Session):
    return db_session.query(Order).all()


def get_open(*, db_session: Session):
    return db_session.query(Order).filter(Order.total > 0)


def list_orders(session: Session):
    result = session.scalars(select(Order).where(Order.total > 0))
    return result.all()


def get_all_with_customers(*, db_session: Session):
    return db_session.query(Order).options(joinedload(Order.customer)).all()


def get_first(*, db_session: Session):
    return db_session.query(Order).first()


def get_built(*, db_session: Session):
    query = db_session.query(Order)
    query = query.options(joinedload(Order.customer))
    return query.all()
