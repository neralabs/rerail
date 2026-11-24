import datetime
import decimal
import uuid

from proj.models import Order, OrderIn
from proj.tasks import not_a_task, run, ship, tag, tag_pickled


def ids(session, order: Order, rows, path, session_id, chat_session):
    ship.delay(order.id)
    ship.apply_async((order.id,), countdown=10)
    ship.delay(session.get(Order, 1).id)
    tag.delay(sorted({"a", "b"}))
    tag.delay(list(set(rows)))
    tag.delay(["a", "b"])
    tag.delay({"a": 1, "b": [1, 2]})
    tag.delay((1, 2))
    tag.delay(datetime.datetime.now(), decimal.Decimal("1.5"), uuid.uuid4(), b"raw")
    tag.delay(path)
    run.delay(session_id)
    run.delay(chat_session.id)
    run.delay(OrderIn(3))
    run.delay(normalize_name)


normalize_name = "proj.fine.normalize"


def rebound(rows):
    unique = set(rows)
    unique = sorted(unique)
    tag.delay(unique)


def pickled(rows):
    tag_pickled.delay({"a"})
    tag_pickled.delay(set(rows))
    tag.apply_async(({"a"},), serializer="pickle")


def not_tasks(order: Order, rows):
    not_a_task.delay({"a"})
    not_a_task.delay(order)
    rows.s(order)
