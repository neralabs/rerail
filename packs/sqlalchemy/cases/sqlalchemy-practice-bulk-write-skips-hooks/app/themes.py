from typing import Optional

from sqlalchemy import Boolean, Column, Integer, String, delete, event, update
from sqlalchemy.orm import Session

from app.db import Base
from app.models import Item

CACHE: dict = {}


class Theme(Base):
    __tablename__ = "theme"
    id = Column(Integer, primary_key=True)
    name = Column(String)
    is_default = Column(Boolean)


@event.listens_for(Theme, "after_insert")
@event.listens_for(Theme, "after_update")
@event.listens_for(Theme, "after_delete")
def clear_theme_cache(_mapper, _connection, _target):
    CACHE.clear()


class Badge(Base):
    __tablename__ = "badge"
    id = Column(Integer, primary_key=True)
    label = Column(String)


@event.listens_for(Badge, "after_update")
def clear_badge_cache(mapper, connection, target):
    CACHE.pop("badges", None)


class SetDefaultTheme:
    def __init__(self, session: Session, theme_id: int):
        self._session = session
        self._theme_id = theme_id
        self._theme: Optional[Theme] = None

    def run(self) -> Theme:
        self._theme = self._session.get(Theme, self._theme_id)
        # the flush of the changed theme below clears the cache anyway
        self._session.execute(update(Theme).where(Theme.is_default.is_(True)).values(is_default=False))
        self._theme.is_default = True
        self._session.add(self._theme)
        return self._theme


class ClearDefaultTheme:
    def __init__(self, session: Session):
        self._session = session

    def run(self) -> None:
        self._session.execute(update(Theme).where(Theme.is_default.is_(True)).values(is_default=False))  # caught


def replace_themes(session: Session, name):
    session.execute(delete(Theme).where(Theme.name == name))
    session.add(Theme(name=name))


def rename_theme(session: Session, theme_id, name):
    theme = session.get(Theme, theme_id)
    theme.name = name
    session.execute(update(Theme).where(Theme.id != theme_id).values(is_default=False))  # caught


def relabel_badges(session: Session, label):
    # the badge listener is registered for updates only: inserting a badge does not run it
    session.execute(update(Badge).values(label=label))  # caught
    session.add(Badge(label=label))


def relabel_items(session: Session, item_id, label):
    # touch_list reads its target: one item's flush does not redo the work for every row
    session.execute(update(Item).values(label=label))  # caught
    item = session.get(Item, item_id)
    item.label = label
