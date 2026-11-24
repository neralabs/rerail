"""Text on PostgreSQL, MEDIUMTEXT/LONGTEXT on MySQL (superset's and dify's shape)."""
from sqlalchemy import Text
from sqlalchemy.dialects.mysql import LONGTEXT, MEDIUMTEXT


def MediumText():  # noqa: N802
    return Text().with_variant(MEDIUMTEXT(), "mysql")


def LongText():  # noqa: N802
    return Text().with_variant(LONGTEXT(), "mysql")
