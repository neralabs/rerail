"""An `op` from another library, in a file shaped like a revision."""
import sqlalchemy as sa

from app.ddl import op

revision = "x1"
down_revision = None


def upgrade():
    op.add_column("users", sa.Column("flag", sa.Boolean(), nullable=False))
