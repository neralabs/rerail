"""teams: a helper that sets another timeout"""
import sqlalchemy as sa
from alembic import op

from migrations.shared.locks import set_statement_timeout

revision = "c12"
down_revision = "c11"
branch_labels = None
depends_on = None


def upgrade():
    set_statement_timeout()
    op.create_check_constraint("ck_teams_size", "teams", "size > 0")  # caught


def downgrade():
    op.drop_constraint("ck_teams_size", "teams", type_="check")
