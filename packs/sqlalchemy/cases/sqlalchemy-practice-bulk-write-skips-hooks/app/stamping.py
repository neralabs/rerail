from datetime import datetime, timezone

from sqlalchemy import Column, DateTime, Integer, String, event, update
from sqlalchemy.orm import Session

from app.db import Base
from app.models import Unit


class Page(Base):
    __tablename__ = "page"
    id = Column(Integer, primary_key=True)
    title = Column(String)
    updated_at = Column(DateTime)
    revision = Column(Integer)


@event.listens_for(Page, "before_update")
def stamp_page(mapper, connection, target):
    """Keep updated_at and the revision current."""
    now = datetime.now(timezone.utc)
    if target.title:
        target.updated_at = now
    target.revision += 1


class Post(Base):
    __tablename__ = "post"
    id = Column(Integer, primary_key=True)
    body = Column(String)
    updated_at = Column(DateTime)


@event.listens_for(Post, "before_update")
def stamp_post(mapper, connection, target):
    target.updated_at = datetime.now(timezone.utc)
    connection.info.setdefault("touched_posts", []).append(target.id)


class Draft(Base):
    __tablename__ = "draft"
    id = Column(Integer, primary_key=True)
    body = Column(String)
    updated_at = Column(DateTime)


@event.listens_for(Draft, "before_update")
def check_draft(mapper, connection, target):
    if not target.body:
        raise ValueError("empty draft")
    target.updated_at = datetime.now(timezone.utc)


def retitle_pages(session: Session, new):
    session.execute(update(Page).values(title=new, updated_at=datetime.now(timezone.utc), revision=Page.revision + 1))


def retitle_pages_unstamped(session: Session, new):
    session.execute(update(Page).values(title=new, revision=Page.revision + 1))  # caught


def rewrite_posts(session: Session, body):
    # the listener also records the post: setting updated_at does not do all of its work
    session.execute(update(Post).values(body=body, updated_at=datetime.now(timezone.utc)))  # caught


def rewrite_drafts(session: Session, body):
    # the listener also refuses empty drafts
    session.execute(update(Draft).values(body=body, updated_at=datetime.now(timezone.utc)))  # caught


def rename_units_normalized(session: Session, new):
    session.execute(update(Unit).values(name=new, normalized_name=new.lower()))


def rename_units_from(session: Session, values):
    session.execute(update(Unit).values(**values))  # caught
