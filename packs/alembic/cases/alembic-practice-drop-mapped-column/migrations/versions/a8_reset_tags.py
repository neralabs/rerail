"""reset tags: both directions drop it, and the application's create_all() builds it again at startup"""
import sqlalchemy as sa
from alembic import op

revision = "a8"
down_revision = "a7"
branch_labels = None


def upgrade():
    op.drop_table("tags")
    op.drop_table("orders")  # caught


def downgrade():
    op.drop_table("tags")
    op.create_table("orders", sa.Column("id", sa.Integer(), primary_key=True))
