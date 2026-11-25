from sqlalchemy import delete, insert, update
from sqlalchemy.orm import Session

from app.models import Item, Note


class ItemRepository:
    model = Item

    def __init__(self, session: Session):
        self.session = session

    def relabel_all(self, label):
        self.session.execute(update(self.model).values(label=label))  # caught


class Repository:
    model = None

    def __init__(self, session: Session):
        self.session = session

    def purge(self):
        self.session.execute(delete(self.model))  # caught


class Items(Repository):
    model = Item


class Notes(Repository):
    model = Note


class Untyped:
    def __init__(self, session: Session, model):
        self.session = session
        self.model = model

    def purge(self):
        self.session.execute(delete(self.model))


def relabel_table(session: Session, old, new):
    session.execute(update(Item.__table__).where(Item.__table__.c.label == old).values(label=new))  # caught


def purge_table(session: Session):
    session.execute(delete(Item.__table__))  # caught


def add_notes(session: Session, rows):
    session.execute(insert(Note.__table__), rows)
