"""copy with a helper that builds sa.update"""
import sqlalchemy as sa
from alembic import op

revision = "b3"
down_revision = "b2"
branch_labels = None
depends_on = None


tag = sa.table("tag", sa.column("data", sa.Text()), sa.column("meta", sa.Text()))


def _copy(bind):
    bind.execute(sa.update(tag).values(meta=tag.c.data))


def upgrade():
    op.add_column("tag", sa.Column("meta", sa.Text()))
    _copy(op.get_bind())
    op.drop_column("tag", "data")


def downgrade():
    op.add_column("tag", sa.Column("data", sa.Text()))
    op.drop_column("tag", "meta")
