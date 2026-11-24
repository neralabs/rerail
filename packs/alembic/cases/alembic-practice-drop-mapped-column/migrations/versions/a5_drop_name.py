"""an older revision drops hero.name; a5b, a later one, adds it back"""
import sqlalchemy as sa
from alembic import op

revision = "a5"
down_revision = "a4"
branch_labels = None


def upgrade():
    op.drop_column("hero", "name")


def downgrade():
    op.add_column("hero", sa.Column("name", sa.String(255)))
