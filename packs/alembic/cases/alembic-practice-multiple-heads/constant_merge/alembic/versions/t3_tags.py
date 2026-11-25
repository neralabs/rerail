"""tags on t1"""
import sqlalchemy as sa
from alembic import op

revision = "t3"
down_revision = "t1"


def upgrade():
    op.create_table("tags", sa.Column("id", sa.Integer(), primary_key=True))


def downgrade():
    op.drop_table("tags")
