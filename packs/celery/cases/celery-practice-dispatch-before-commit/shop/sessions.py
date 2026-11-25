import aiohttp
import requests
from sqlalchemy.orm import Session

from shop.db import SessionLocal, engine
from shop.tasks import notify, ship


# ---- reported: a SQLAlchemy session under any name -----------------------------------------------------------------


def with_alias(order):
    with Session(engine) as db:
        db.add(order)
        ship.delay(order.id)  # caught
        db.commit()


def from_factory(order):
    db = SessionLocal()
    db.add(order)
    notify.delay(order.id)  # caught
    db.commit()


def annotated(db: Session, order):
    db.merge(order)
    ship.delay(order.id)  # caught
    db.commit()


# ---- silent: HTTP sessions with the same method names --------------------------------------------------------------


def purge_remote(http_session: requests.Session, db_session, order):
    db_session.add(order)
    db_session.commit()
    http_session.delete(f"https://cdn.example/{order.id}")
    notify.delay(order.id)
    http_session.commit()


def purge_local_client(db_session, order):
    db_session.add(order)
    db_session.commit()
    session = requests.Session()
    session.delete(f"https://cdn.example/{order.id}")
    notify.delay(order.id)
    session.commit()


async def purge_async(client: aiohttp.ClientSession, db_session, order):
    db_session.add(order)
    db_session.commit()
    await client.delete(f"https://cdn.example/{order.id}")
    notify.delay(order.id)
    client.commit()
