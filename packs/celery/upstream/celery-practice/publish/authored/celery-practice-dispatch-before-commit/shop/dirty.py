from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession
from sqlalchemy.orm import Session

from shop.models import Invoice, Order, OrderView
from shop.tasks import notify, ship

# A column assigned on a row the function loaded through its session is a write: the commit flushes it.


# ---- reported ------------------------------------------------------------------------------------------------------


def mark_paid(session: Session, order_id):
    order = session.get(Order, order_id)
    order.status = "paid"
    ship.delay(order.id)  # caught
    session.commit()


def mark_paid_query(session, order_id):
    order = session.query(Order).filter(Order.id == order_id).first()
    order.paid = True
    notify.delay(order.id)  # caught
    session.commit()


def mark_paid_select(session: Session, order_id):
    order = session.scalar(select(Order).where(Order.id == order_id))
    order.status = "paid"
    ship.delay(order.id)  # caught
    session.commit()


def touch_inherited_column(session: Session, order_id, now):
    order = session.execute(select(Order).where(Order.id == order_id)).scalar_one()
    order.updated_at = now
    ship.delay(order.id)  # caught
    session.commit()


async def mark_paid_async(session: AsyncSession, order_id):
    order = await session.get(Order, order_id)
    order.status = "paid"
    notify.delay(order.id)  # caught
    await session.commit()


def mark_all_paid(session: Session):
    for order in session.scalars(select(Order).where(Order.status == "new")):
        order.status = "paid"
    ship.delay(0)  # caught
    session.commit()


def close_invoice(session: Session, invoice_id):
    invoice = session.query(Invoice).get(invoice_id)
    invoice.state = "closed"
    notify.delay(invoice.id)  # caught
    session.commit()


# ---- silent: no column of a loaded row changes ---------------------------------------------------------------------


def set_plain_attribute(session: Session, order_id):
    order = session.get(Order, order_id)
    order.note = "seen"
    ship.delay(order.id)
    session.commit()


def set_class_attribute(session: Session, invoice_id):
    invoice = session.get(Invoice, invoice_id)
    invoice.label = "paid invoice"
    notify.delay(invoice.id)
    session.commit()


def change_new_object(session: Session, data):
    order = Order(**data)
    order.status = "draft"
    ship.delay(order.id)
    session.commit()


def change_a_view(session: Session, view: OrderView):
    view.status = "paid"
    ship.delay(view.id)
    session.commit()


def read_then_send(session: Session, order_id):
    order = session.get(Order, order_id)
    status = order.status
    notify.delay(status)
    session.commit()


def change_from_cache(session: Session, cache, order_id):
    order = cache.get(Order, order_id)
    order.status = "paid"
    ship.delay(order.id)
    session.commit()


def change_then_commit(session: Session, order_id):
    order = session.get(Order, order_id)
    order.status = "paid"
    session.commit()
    ship.delay(order.id)
