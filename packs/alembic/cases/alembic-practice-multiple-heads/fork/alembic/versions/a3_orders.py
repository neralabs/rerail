"""add orders: written on a2, a head"""
from typing import Sequence, Union

import sqlalchemy as sa
from alembic import op

revision: str = "a3"
down_revision: Union[str, None] = "a2"  # caught
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None

def upgrade():
    op.create_table("orders", sa.Column("id", sa.Integer(), primary_key=True))


def downgrade():
    pass
