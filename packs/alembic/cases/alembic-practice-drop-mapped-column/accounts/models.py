"""A third service; its env.py imports no models, so its tree's models are those under accounts/."""
from sqlalchemy import Boolean, Column, Integer, MetaData, String, Table
from sqlalchemy.orm import declarative_base

from accounts.tables import sessions_table

Base = declarative_base()
md = MetaData()

profiles_table = Table(
    "profiles", md, Column("id", Integer, primary_key=True), Column("bio", String), Column("avatar", String)
)
prefs_table = Table("prefs", md, Column("id", Integer, primary_key=True), Column("theme", String))


class Profile(Base):
    __table__ = profiles_table
    __mapper_args__ = {"exclude_properties": ["bio"]}


class Prefs(Base):
    __table__ = prefs_table
    __mapper_args__ = dict(exclude_properties=[prefs_table.c.theme])


class Session(Base):
    # A Table imported from another module, mapped with only the listed columns.
    __table__ = sessions_table
    __mapper_args__ = {"include_properties": ("id", "token")}


class ArchiveMixin:
    __mapper_args__ = {"exclude_properties": {"old_flag"}}


class Account(ArchiveMixin, Base):
    # The mixin's __mapper_args__ applies: old_flag is not mapped.
    __tablename__ = "accounts"
    id = Column(Integer, primary_key=True)
    plan = Column(String(20))
    old_flag = Column(Boolean)


class Archived(ArchiveMixin, Base):
    # Its own __mapper_args__ replaces the mixin's: old_flag is mapped here.
    __tablename__ = "archived"
    __mapper_args__ = {"exclude_properties": ["note"]}
    id = Column(Integer, primary_key=True)
    old_flag = Column(Boolean)
    note = Column(String)
