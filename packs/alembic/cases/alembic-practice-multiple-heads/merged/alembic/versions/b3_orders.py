"""add orders"""
import sqlalchemy as sa
from alembic import op

revision = "b3"
down_revision = "b1"
branch_labels = None

def upgrade():
    op.create_table("orders", sa.Column("id", sa.Integer(), primary_key=True))


def downgrade():
    pass
