"""tags on s1, named through a module constant"""
import sqlalchemy as sa
from alembic import op

PARENT = "s1"

revision = "s3"
down_revision = PARENT  # caught


def upgrade():
    op.create_table("tags", sa.Column("id", sa.Integer(), primary_key=True))


def downgrade():
    op.drop_table("tags")
