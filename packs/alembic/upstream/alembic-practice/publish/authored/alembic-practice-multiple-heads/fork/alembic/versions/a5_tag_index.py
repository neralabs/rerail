"""index tags: the head of a4's line"""
import sqlalchemy as sa
from alembic import op

revision = 'a5'
down_revision = 'a4'  # caught
branch_labels = None
depends_on = None

def upgrade():
    op.create_index("ix_tags_id", "tags", ["id"])


def downgrade():
    pass
