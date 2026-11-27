"""projects: TO DEFAULT is the server's default, off"""
import sqlalchemy as sa
from alembic import op

revision = "c18"
down_revision = "c17"
branch_labels = None
depends_on = None


def upgrade():
    op.execute(sa.text("SET lock_timeout TO DEFAULT"))
    op.drop_column("projects", "nick")  # caught


def downgrade():
    pass
