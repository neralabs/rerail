from sqlalchemy import update

from shop.models import Order
from shop.tasks import notify, ship


class OrderService:
    @staticmethod
    def _apply(session, order_id, values):
        session.execute(update(Order).where(Order.id == order_id).values(**values))
        session.flush()
        ship.delay(order_id)  # caught

    @staticmethod
    def update(session, order_id, values):
        return OrderService._apply(session, order_id, values)


def stage_and_notify(session, order):
    session.add(order)
    notify.delay(order.id)  # caught


def save_and_queue(session, order):
    session.add(order)
    notify.delay(order.id)


def write_and_queue_no_committer(session, order):
    session.add(order)
    ship.delay(order.id)
