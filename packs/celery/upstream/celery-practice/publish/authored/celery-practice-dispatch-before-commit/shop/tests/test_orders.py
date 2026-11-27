from shop.models import Order
from shop.tasks import ship


def test_create(session):
    order = Order()
    session.add(order)
    ship.delay(order.id)
    session.commit()
