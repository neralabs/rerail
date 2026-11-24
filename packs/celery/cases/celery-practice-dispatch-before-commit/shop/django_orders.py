from django.db import transaction

from shop.models import Order
from shop.tasks import ship


def create(session):
    order = Order()
    session.add(order)
    ship.delay(order.id)
    session.commit()
    transaction.on_commit(lambda: None)
