"""email on x1; its own id and its parent are module constants"""
import sqlalchemy as sa
from alembic import op

REVISION_ID = "x2"
PARENT_ID = "x1"

revision = REVISION_ID
down_revision = PARENT_ID  # caught


def upgrade():
    op.add_column("users", sa.Column("email", sa.String(255)))


def downgrade():
    op.drop_column("users", "email")
