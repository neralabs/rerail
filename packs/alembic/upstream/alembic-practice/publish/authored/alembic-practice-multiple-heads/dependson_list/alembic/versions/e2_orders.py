"""orders on e1"""
from typing import Sequence, Union

import sqlalchemy as sa
from alembic import op

revision: str = "e2"
down_revision: Union[str, None] = "e1"  # caught
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None


def upgrade():
    op.create_table("orders", sa.Column("id", sa.Integer(), primary_key=True))


def downgrade():
    op.drop_table("orders")
