"""drop server defaults the models no longer declare"""
import sqlalchemy as sa
from alembic import op

revision = "n6"
down_revision = "n5"
branch_labels = None
depends_on = None


def upgrade():
    op.alter_column("trial_apps", "id", server_default=None)


def downgrade():
    # The down revision's code never relied on the default, and dropping it again is a no-op.
    pass
