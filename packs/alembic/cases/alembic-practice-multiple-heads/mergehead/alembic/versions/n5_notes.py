"""notes, written on n2 before the merge landed"""
import sqlalchemy as sa
from alembic import op

revision = "n5"
down_revision = "n2"  # caught
branch_labels = None
depends_on = None


def upgrade():
    op.create_table("notes", sa.Column("id", sa.Integer(), primary_key=True))


def downgrade():
    pass
