from sqlalchemy.orm import Session

from shop.db import SessionLocal, engine
from shop.tasks import notify, ship

audit_session = SessionLocal()


# ---- reported: the caller commits the session the helper wrote to --------------------------------------------------


def stage(order, *, session):
    session.add(order)
    ship.delay(order.id)  # caught


def create_keyword(order):
    with Session(engine) as db:
        stage(order, session=db)
        db.commit()


class Orders:
    def __init__(self, session: Session):
        self.session = session

    def _stage(self, session, order):
        session.add(order)
        notify.delay(order.id)  # caught

    def create(self, order):
        self._stage(self.session, order)
        self.session.commit()


def log_event(order):
    audit_session.add(order)
    notify.delay(order.id)  # caught


def create_logged(order):
    log_event(order)
    audit_session.commit()


# ---- silent: the caller commits another session ---------------------------------------------------------------------


def stage_audit(other_session, order):
    other_session.add(order)
    ship.delay(order.id)


def create_two_sessions(db: Session, order):
    with Session(engine) as audit_db:
        stage_audit(audit_db, order)
    db.commit()
