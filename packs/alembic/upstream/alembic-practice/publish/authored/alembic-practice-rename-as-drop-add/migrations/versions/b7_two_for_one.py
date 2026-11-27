"""replace one timestamp with two"""
import sqlalchemy as sa
from alembic import op

revision = "b7"
down_revision = "b6"
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("case", sa.Column("triage_at", sa.DateTime(), nullable=True))
    op.add_column("case", sa.Column("escalated_at", sa.DateTime(), nullable=True))
    op.drop_column("case", "stable_at")


def downgrade():
    op.add_column("case", sa.Column("stable_at", sa.TIMESTAMP(), autoincrement=False, nullable=True))
    op.drop_column("case", "escalated_at")
    op.drop_column("case", "triage_at")
