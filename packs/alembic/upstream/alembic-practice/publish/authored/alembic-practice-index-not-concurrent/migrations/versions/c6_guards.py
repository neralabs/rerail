"""dialect guards that return early"""
import sqlalchemy as sa
from alembic import op

revision = "c6"
down_revision = "c5"
branch_labels = None


def _is_mysql() -> bool:
    return op.get_context().dialect.name == "mysql"


def upgrade() -> None:
    _mysql_only()
    _pg_only()


def _mysql_only():
    # Everything after this guard runs on MySQL only.
    if not _is_mysql():
        return
    op.add_column("jobs", sa.Column("active_id", sa.String(36), nullable=True))
    op.create_index("ix_jobs_active_id", "jobs", ["tenant_id", "active_id"], unique=True)


def _pg_only():
    # Everything after this guard runs on PostgreSQL: still a blocking build.
    if op.get_bind().dialect.name != "postgresql":
        return
    op.create_index("ix_jobs_payload", "jobs", ["payload"], postgresql_using="gin")  # caught


def downgrade() -> None:
    pass
