"""plain: a new table locks nothing"""
import sqlalchemy as sa
from alembic import op

revision = "p3"
down_revision = 'p2'
branch_labels = None
depends_on = None


def upgrade():
    op.create_table("tags", sa.Column("id", sa.Integer(), primary_key=True))
    op.create_index("ix_tags_id", "tags", ["id"])


def downgrade():
    pass
