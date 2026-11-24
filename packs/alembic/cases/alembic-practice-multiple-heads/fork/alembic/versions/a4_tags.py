"""add tags: also written on a2, not a head itself"""
import sqlalchemy as sa
from alembic import op

revision = 'a4'
down_revision = 'a2'
branch_labels = None
depends_on = None

def upgrade():
    op.create_table("tags", sa.Column("id", sa.Integer(), primary_key=True))


def downgrade():
    pass
