from alembic import op
from sqlalchemy import inspect


def create_table(table_name, *columns, **kwargs):
    """Create the table unless it is already there (a project's own wrapper of op.create_table)."""
    if inspect(op.get_bind()).has_table(table_name):
        return
    op.create_table(table_name, *columns, **kwargs)
