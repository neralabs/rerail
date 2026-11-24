"""tags"""
import sqlalchemy as sa
from alembic import op

revision = "n3"
down_revision = "n1"
branch_labels = None
depends_on = None


def upgrade():
    op.create_table("tags", sa.Column("id", sa.Integer(), primary_key=True))


def downgrade():
    pass
