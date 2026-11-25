"""tags on v2"""
import sqlalchemy as sa
from alembic import op

revision = "v3"
down_revision = "v2"


def upgrade():
    op.create_table("tags", sa.Column("id", sa.Integer(), primary_key=True))


def downgrade():
    op.drop_table("tags")
