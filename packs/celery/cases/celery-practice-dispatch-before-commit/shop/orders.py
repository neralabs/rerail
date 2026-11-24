from sqlalchemy import event, update
from sqlalchemy.orm import Session

from shop.celery import app
from shop.db import SessionLocal
from shop.helpers import queue_after_own_commit, queue_deep, queue_shipping
from shop.models import Order
from shop.tasks import notify, ship


def create_order(session: Session, data):
    order = Order(**data)
    session.add(order)
    session.flush()
    ship.delay(order.id)  # caught
    session.commit()


def mark_paid(session: Session, order_id):
    session.execute(update(Order).where(Order.id == order_id).values(paid=True))
    ship.apply_async(args=[order_id], countdown=5)  # caught
    app.send_task("shop.tasks.notify", args=[order_id])  # caught
    session.commit()


def branchy(session: Session, order, express):
    session.add(order)
    if express:
        ship.delay(order.id)  # caught
    else:
        notify.delay(order.id)  # caught
    session.commit()


def in_begin(data):
    with SessionLocal.begin() as session:
        order = Order(**data)
        session.add(order)
        session.flush()
        ship.delay(order.id)  # caught


async def in_async_begin(async_session, order):
    async with async_session.begin():
        async_session.add(order)
        notify.delay(order.id)  # caught


def via_helper(session, order):
    session.add(order)
    session.flush()
    queue_shipping(order.id)  # caught
    session.commit()


def via_deep_helper(session, order):
    session.add(order)
    queue_deep(order.id)  # caught
    session.commit()


def helper_in_begin(order):
    with SessionLocal.begin() as session:
        session.add(order)
        queue_shipping(order.id)  # caught


# ---- near misses: silent ----------------------------------------------------------------------------------------


def create_then_send(session: Session, data):
    order = Order(**data)
    session.add(order)
    session.commit()
    ship.delay(order.id)


def commit_between(session, order):
    session.add(order)
    session.commit()
    ship.delay(order.id)
    session.add(Order())
    session.commit()


def after_begin(data):
    with SessionLocal.begin() as session:
        order = Order(**data)
        session.add(order)
    ship.delay(order.id)


def read_only_begin():
    with SessionLocal.begin() as session:
        ids = [o.id for o in session.query(Order).all()]
    for i in ids:
        ship.delay(i)


def failing(session, order):
    try:
        session.add(order)
        session.flush()
    except Exception:
        notify.delay(order.id)
        raise
    session.commit()


def result_used(session, order):
    session.add(order)
    result = ship.apply_async(args=[order.id])
    order.task_id = result.id
    session.commit()


def with_after_commit(session, order):
    session.add(order)
    event.listen(session, "after_commit", lambda s: ship.delay(order.id), once=True)

    def _send(s):
        notify.delay(order.id)

    event.listen(session, "after_commit", _send, once=True)
    session.commit()


def not_a_task(session, order, throttle):
    session.add(order)
    throttle.delay(2.0)
    session.commit()


def via_committing_helper(session, order):
    session.add(order)
    queue_after_own_commit(session, order.id)
    session.commit()


def no_commit_anywhere(session, order):
    session.add(order)
    ship.delay(order.id)
