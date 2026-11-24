from alembic import op
from sqlalchemy import text


def upgrade() -> None:
    conn = op.get_bind()
    conn.execute(text("UPDATE users SET name = '' WHERE name IS NULL"))


def downgrade() -> None:
    pass
