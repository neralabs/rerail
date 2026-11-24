import sqlalchemy as sa
from alembic import op

revision = "t1"
down_revision = "b2"


def upgrade():
    op.alter_column("users", "age", existing_type=sa.Integer(), type_=sa.BigInteger())
