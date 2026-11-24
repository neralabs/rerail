from alembic import op
from sqlalchemy import inspect, text

revision = "0002"
down_revision = "0001"


def upgrade():
    bind = op.get_bind()
    for table in inspect(bind).get_table_names():
        bind.execute(text(f"UPDATE {table} SET owner = 'system' WHERE owner IS NULL"))
