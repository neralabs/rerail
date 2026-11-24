"""validate c1's constraints; a table created through the project's create_table wrapper"""
import sqlalchemy as sa
from alembic import op

from migrations.shared.utils import create_table

revision = "c3"
down_revision = "c2"
branch_labels = None


def _is_mysql() -> bool:
    return op.get_context().dialect.name == "mysql"


def upgrade():
    op.execute("ALTER TABLE orders VALIDATE CONSTRAINT fk_orders_agent")
    op.execute("ALTER TABLE orders VALIDATE CONSTRAINT ck_orders_paid")
    create_table(
        "reports",
        sa.Column("id", sa.Integer(), primary_key=True),
        sa.Column("owner_id", sa.Integer(), nullable=False),
    )
    op.create_foreign_key("fk_reports_owner", "reports", "users", ["owner_id"], ["id"])
    _mysql_only()
    _pg_only()


def _mysql_only():
    # Everything after this guard runs on MySQL only.
    if not _is_mysql():
        return
    op.create_check_constraint("ck_jobs_state", "jobs", "state in ('a', 'b')")


def _pg_only():
    # Everything after this guard runs on PostgreSQL.
    if op.get_bind().dialect.name != "postgresql":
        return
    op.create_check_constraint("ck_jobs_payload", "jobs", "jsonb_typeof(payload) = 'object'")  # caught


def downgrade():
    op.drop_constraint("fk_reports_owner", "reports", type_="foreignkey")
