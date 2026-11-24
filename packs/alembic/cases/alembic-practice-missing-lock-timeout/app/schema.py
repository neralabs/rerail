"""Not a revision: no revision/down_revision."""
import sqlalchemy as sa
from alembic import op


def add_audit_column(table):
    op.add_column("audit", sa.Column("actor", sa.String(50)))
