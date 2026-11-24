"""merge b6 and b7 (a list)"""
from alembic import op

revision = "b8"
down_revision = ["b6", "b7"]
branch_labels = None


def upgrade():
    pass


def downgrade():
    pass
