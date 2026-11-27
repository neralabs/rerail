from sqlalchemy import event

from shop.db import SessionLocal
from shop.helpers import queue_shipping
from shop.models import Order
from shop.tasks import notify


@event.listens_for(Order, "after_insert")
def order_inserted(mapper, connection, target):
    notify.delay(target.id)  # caught


def order_updated(mapper, connection, target):
    queue_shipping(target.id)  # caught


event.listen(Order, "after_update", order_updated)
event.listen(Order, "after_delete", order_updated)


@event.listens_for(SessionLocal, "after_flush")
def flushed(session, flush_context):
    for obj in session.new:
        notify.delay(obj.id)  # caught


@event.listens_for(SessionLocal, "after_commit")
def committed(session):
    notify.delay(1)


@event.listens_for(Order, "load")
def loaded(target, context):
    notify.delay(target.id)
