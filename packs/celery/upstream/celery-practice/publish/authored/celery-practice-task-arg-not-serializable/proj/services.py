from sqlalchemy import select
from sqlalchemy.orm import Session

from proj.celery import app
from proj.models import Order
from proj.tasks import run, ship, tag, tag_pickled


def normalize(row):
    return row


def place(session: Session, pk: int):
    order = session.get(Order, pk)
    ship.delay(order)  # caught
    ship.apply_async((session.scalar(select(Order).where(Order.id == pk)),))  # caught
    ship.apply_async(kwargs={"order_id": Order(id=pk)})  # caught
    ship.s(order)  # caught
    ship.delay(session)  # caught
    tag_pickled.delay(order)  # caught


def by_annotation(order: Order):
    ship.si(order)  # caught


def legacy(pk):
    first = Order.query.filter_by(id=pk).first()
    ship.delay(order_id=first)  # caught


def sets(ids):
    tag.delay({"a", "b"})  # caught
    tag.delay({i for i in ids})  # caught
    tag.delay(set(ids))  # caught
    tag.delay(tags=frozenset(ids))  # caught
    unique = set(ids)
    tag.delay(unique)  # caught
    app.send_task("proj.tasks.tag", args=[{1, 2}])  # caught


def others(path, rows):
    tag.delay(lambda r: r)  # caught
    tag.delay(r for r in rows)  # caught
    tag.delay(open(path))  # caught
    fh = open(path)
    run.delay(fh)  # caught
    with open(path) as f:
        run.delay(f)  # caught
    run.delay(normalize)  # caught
    run.apply_async(args=(normalize,))  # caught
