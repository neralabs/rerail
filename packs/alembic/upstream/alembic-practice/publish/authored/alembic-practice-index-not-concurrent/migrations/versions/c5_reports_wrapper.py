"""reports: a table created through the project's create_table wrapper"""
import sqlalchemy as sa
from alembic import op

from migrations.shared.utils import create_table

revision = "c5"
down_revision = "c4"
branch_labels = None


def upgrade():
    create_table(
        "reports",
        sa.Column("id", sa.Integer(), primary_key=True),
        sa.Column("owner_id", sa.Integer(), nullable=False),
    )
    op.create_index(op.f("ix_reports_owner_id"), "reports", ["owner_id"], unique=False)
    op.create_unique_constraint("uq_reports_owner", "reports", ["owner_id", "id"])
    op.create_index(op.f("ix_users_report_id"), "users", ["report_id"])  # caught


def downgrade():
    op.drop_index(op.f("ix_users_report_id"), table_name="users")
    op.drop_table("reports")
