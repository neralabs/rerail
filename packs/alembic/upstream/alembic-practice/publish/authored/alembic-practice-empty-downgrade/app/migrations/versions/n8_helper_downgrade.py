"""rename column, reversed by a helper"""
import sqlalchemy as sa
from alembic import op

revision = "n8"
down_revision = "n7"
branch_labels = None
depends_on = None


def _rename(a, b):
    op.alter_column("users", a, new_column_name=b)


def upgrade():
    _rename("name", "full_name")


def downgrade():
    _rename("full_name", "name")
