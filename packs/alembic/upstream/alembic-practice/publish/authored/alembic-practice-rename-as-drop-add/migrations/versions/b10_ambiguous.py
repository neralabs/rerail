"""two adds and two drops of one type: no pairing to name"""
import sqlalchemy as sa
from alembic import op

revision = "b10"
down_revision = "b9"
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("profile", sa.Column("first_name", sa.String(60)))
    op.add_column("profile", sa.Column("last_name", sa.String(60)))
    op.drop_column("profile", "given")
    op.drop_column("profile", "family")


def downgrade():
    op.add_column("profile", sa.Column("family", sa.VARCHAR(length=60)))
    op.add_column("profile", sa.Column("given", sa.VARCHAR(length=60)))
    op.drop_column("profile", "last_name")
    op.drop_column("profile", "first_name")
