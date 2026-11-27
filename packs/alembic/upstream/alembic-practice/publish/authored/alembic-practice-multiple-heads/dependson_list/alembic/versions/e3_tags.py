"""tags on e1, depends_on as a list, no branch_labels"""
from typing import Sequence, Union

import sqlalchemy as sa
from alembic import op

revision: str = "e3"
down_revision: Union[str, None] = "e1"  # caught
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = ["e2"]


def upgrade():
    op.create_table("tags", sa.Column("id", sa.Integer(), primary_key=True))


def downgrade():
    op.drop_table("tags")
