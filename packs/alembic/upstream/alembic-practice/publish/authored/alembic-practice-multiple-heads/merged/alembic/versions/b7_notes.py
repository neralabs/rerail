"""add notes"""
import sqlalchemy as sa
from alembic import op

revision = "b7"
down_revision = "b5"
branch_labels = None

def upgrade():
    op.create_table("notes", sa.Column("id", sa.Integer(), primary_key=True))


def downgrade():
    pass
