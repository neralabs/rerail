import sqlalchemy as sa
from alembic import op

revision = "t1"
down_revision = "a2"


def upgrade():
    op.add_column("users", sa.Column("flag", sa.Boolean(), nullable=False))
