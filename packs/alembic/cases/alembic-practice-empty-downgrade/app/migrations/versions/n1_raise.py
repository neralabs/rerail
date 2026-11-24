"""drop legacy audit table"""
import sqlalchemy as sa
from alembic import op

revision = "n1"
down_revision = "c4"
branch_labels = None
depends_on = None


def upgrade():
    op.drop_table("legacy_audit")


def downgrade():
    raise NotImplementedError("irreversible: legacy_audit's rows are gone")
