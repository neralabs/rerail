from sqlalchemy.orm import Session

from shop.db import SessionLocal, engine
from shop.services import OrderService, save_and_queue, stage_and_notify, write_and_queue_no_committer


def update_order(order_id, values):
    with Session(engine) as session:
        OrderService.update(session, order_id, values)
        session.commit()


def create(order):
    with SessionLocal.begin() as session:
        stage_and_notify(session, order)


def create_committed_first(session, order):
    session.commit()
    save_and_queue(session, order)


def create_no_commit(session, order):
    write_and_queue_no_committer(session, order)
