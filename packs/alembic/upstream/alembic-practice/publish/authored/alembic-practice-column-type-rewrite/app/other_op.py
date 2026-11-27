"""An `op` from another library, in a file shaped like a revision."""
import sqlalchemy as sa

from app.ddl import op

revision = "x1"
down_revision = None


def upgrade():
    op.alter_column("users", "age", existing_type=sa.Integer(), type_=sa.BigInteger())
