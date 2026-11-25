"""projects: set_config to zero"""
import sqlalchemy as sa
from alembic import op

revision = "c19"
down_revision = "c18"
branch_labels = None
depends_on = None


def upgrade():
    op.execute("SELECT set_config('lock_timeout', '0', true)")
    op.alter_column("projects", "name", existing_type=sa.String(50), nullable=False)  # caught


def downgrade():
    pass
