"""add tags after the merge"""
import sqlalchemy as sa
from alembic import op

revision = "b5"
down_revision = "b4"
branch_labels = None

def upgrade():
    op.create_table("tags", sa.Column("id", sa.Integer(), primary_key=True))


def downgrade():
    pass
