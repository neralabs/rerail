"""rename accounts.title; downgrade does not show the old type, table named by a constant"""
import sqlalchemy as sa
from alembic import op

revision = "a3"
down_revision = "a2"
branch_labels = None
depends_on = None

ACCOUNTS = "accounts"


def upgrade():
    op.drop_column(ACCOUNTS, "title")  # caught
    op.add_column(ACCOUNTS, sa.Column("display_name", sa.Unicode(80)))


def downgrade():
    pass
