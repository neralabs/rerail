"""Project types: Text on PostgreSQL, MEDIUMTEXT/LONGTEXT on MySQL (superset's and dify's shape), and others."""
import sqlalchemy as sa
from sqlalchemy import Text
from sqlalchemy.dialects.mysql import LONGTEXT, MEDIUMTEXT
from sqlalchemy.types import TypeDecorator


def MediumText():  # noqa: N802
    return Text().with_variant(MEDIUMTEXT(), "mysql")


def LongText():  # noqa: N802
    return Text().with_variant(LONGTEXT(), "mysql")


class StringUUID(TypeDecorator):
    """A UUID on PostgreSQL, CHAR(36) elsewhere (dify's shape)."""

    impl = sa.UUID
    cache_ok = True


class Email(sa.String):
    """A VARCHAR with a project name."""


Int32 = sa.Integer
