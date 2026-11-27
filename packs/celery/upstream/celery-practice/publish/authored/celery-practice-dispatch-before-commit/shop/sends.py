from multiprocessing import Pool

from celery import signature
from rq.decorators import job as background

from shop.jobs import ship as exported_ship
from shop.jobs.billing import refund, refund_task
from shop.pool import POOL
from shop.rq_jobs import reindex
from shop.tasks import ship as ship_order


class Limiter:
    def delay(self, seconds):
        return seconds


LIMITER = Limiter()


@background("low")
def reindex_later(order_id):
    return order_id


# ---- reported: Celery tasks however they are reached ---------------------------------------------------------------


def via_alias(session, order):
    session.add(order)
    ship_order.delay(order.id)  # caught
    session.commit()


def via_reexport(session, order):
    session.add(order)
    exported_ship.apply_async((order.id,))  # caught
    session.commit()


def via_task_object(session, order):
    session.add(order)
    refund.delay(order.id)  # caught
    session.commit()


def via_task_instance(session, order):
    session.add(order)
    refund_task.delay(order.id)  # caught
    session.commit()


def via_signature(session, order):
    session.add(order)
    ship_order.s(order.id).apply_async()  # caught
    signature("shop.tasks.notify", args=(order.id,)).delay()  # caught
    session.commit()


# ---- silent: the same method names on things that are not Celery tasks --------------------------------------------


def rq_dispatch(session, order):
    session.add(order)
    reindex.delay(order.id)
    reindex_later.delay(order.id)
    session.commit()


def pool_dispatch(session, order):
    session.add(order)
    POOL.apply_async(print, (order.id,))
    with Pool(2) as pool:
        pool.apply_async(print, (order.id,))
    session.commit()


def limiter_delay(session, order):
    session.add(order)
    LIMITER.delay(0.5)
    session.commit()
