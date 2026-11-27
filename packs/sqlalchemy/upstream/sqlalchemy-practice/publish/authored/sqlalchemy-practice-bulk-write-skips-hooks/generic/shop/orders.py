from sqlalchemy import delete, update
from sqlalchemy.orm import Session

from shop.models import Order


def cancel_all(session: Session):
    session.execute(update(Order).values(status="cancelled"))  # caught


def purge(session: Session):
    session.execute(delete(Order))
