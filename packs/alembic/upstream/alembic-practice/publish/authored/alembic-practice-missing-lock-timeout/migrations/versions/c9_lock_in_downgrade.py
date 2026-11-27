"""archive: lock_timeout set in downgrade, not upgrade"""
import sqlalchemy as sa
from alembic import op

revision = "c9"
down_revision = "c8"
branch_labels = None
depends_on = None


def upgrade():
    op.drop_table("archive_entries")  # caught


def downgrade():
    op.execute("SET LOCAL lock_timeout = '5s'")
    op.create_table("archive_entries", sa.Column("id", sa.Integer(), primary_key=True))
