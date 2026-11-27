"""tags on r1"""
import sqlalchemy as sa
from alembic import op

revision = "r3"
down_revision = "r1"  # caught


def upgrade():
    op.create_table("tags", sa.Column("id", sa.Integer(), primary_key=True))


def downgrade():
    op.drop_table("tags")
