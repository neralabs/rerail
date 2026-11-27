"""labels: the lock in a module-level helper upgrade() calls"""
import sqlalchemy as sa
from alembic import op

revision = "c14"
down_revision = "c13"
branch_labels = None
depends_on = None


def _add_label_columns():
    op.add_column("labels", sa.Column("color", sa.String(7), nullable=True))  # caught
    op.create_unique_constraint("uq_labels_name", "labels", ["name"])


def upgrade():
    _add_label_columns()


def downgrade():
    op.drop_column("labels", "color")
