from sqlalchemy import select

from app.models import Order


def test_customers(session):
    for order in session.scalars(select(Order)):
        assert order.customer is not None
