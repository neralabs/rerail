from alembic import op
from sqlalchemy import inspect


def create_table(table_name, *columns, **kwargs):
    """Create the table unless it is already there (a project's own wrapper of op.create_table)."""
    if inspect(op.get_bind()).has_table(table_name):
        return
    op.create_table(table_name, *columns, **kwargs)


def create_index(table_name, index_name, columns, *, unique=False):
    """Create the index unless it is already there (superset's shared helper): no CONCURRENTLY."""
    if index_name in {i["name"] for i in inspect(op.get_bind()).get_indexes(table_name)}:
        return
    op.create_index(table_name=table_name, index_name=index_name, columns=columns, unique=unique)


def create_index_concurrently(table_name, index_name, columns):
    """The concurrent form of the same helper."""
    with op.get_context().autocommit_block():
        op.create_index(index_name, table_name, columns, postgresql_concurrently=True)
