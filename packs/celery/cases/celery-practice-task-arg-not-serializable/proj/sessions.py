import uuid

import aiohttp
import requests
from aiohttp import ClientSession
from flask_login import current_user
from sqlalchemy.orm import Session

from proj import db as database
from proj.db import ScopedSession, SessionDep, SessionLocal, db, engine, get_session, new_session
from proj.models import Order
from proj.rq_jobs import POOL, compute, rq_tag
from proj.tasks import run, ship, tag


# ---- SQLAlchemy sessions, proven by what binds them ----
def bound_sessions(pk):
    with Session(engine) as session:
        run.delay(session)  # caught
    local = SessionLocal()
    run.delay(local)  # caught
    run.delay(db.session)  # caught
    run.delay(database.db.session)  # caught
    run.delay(ScopedSession)  # caught
    made = new_session()
    run.delay(made)  # caught
    with SessionLocal.begin() as tx:
        run.delay(tx)  # caught


def dependency(session: SessionDep, sess: Session):
    run.delay(session)  # caught
    run.apply_async(args=(sess,))  # caught


def unannotated(session, rows):
    # Nothing binds `session` here: the name is all there is, in a module that uses SQLAlchemy.
    run.delay(session)  # caught


# ---- models inside containers and keyword spreads, and a proxy of the user model ----
def containers(pk, tags):
    order = SessionLocal().get(Order, pk)
    ship.delay([order])  # caught
    ship.delay(orders=(order, order))  # caught
    ship.delay(**{"order": order})  # caught
    ship.apply_async(kwargs=dict(order=order))  # caught
    ship.delay(current_user)  # caught
    tag.delay(tags | {"x"})  # caught
    tag.delay(set(tags) - {"x"})  # caught


# ---- near misses: values named like a session that are not one ----
def chat(request):
    session = request.json["session"]
    run.delay(session)
    session = str(uuid.uuid4())
    run.delay(session)


def chat_dict():
    session = {"user": 1, "turns": []}
    run.delay(session)


def http(url):
    with requests.Session() as session:
        run.delay(session.get(url).text)
    client_session = aiohttp.ClientSession()
    run.delay(client_session.headers.get("x"))


def typed_http(session: ClientSession, session_key: str):
    run.delay(session_key)


def from_dependency():
    for session in get_session():
        run.delay(session.info.get("tenant"))


# ---- near misses: other libraries' .delay and .apply_async ----
def look_alikes(tags):
    rq_tag.delay(set(tags))
    POOL.apply_async(compute, ({"a"},))


# ---- a session kept on self: proven through the class's own assignment ----
class Repository:
    def __init__(self, session: Session, client: ClientSession):
        self.session = session
        self.http_session = client

    def enqueue(self):
        run.delay(self.session)  # caught
        run.delay(self.http_session)
