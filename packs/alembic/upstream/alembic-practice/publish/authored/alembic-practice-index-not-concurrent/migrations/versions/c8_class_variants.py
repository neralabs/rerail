"""the same shapes through aliases, constants and re-exported helpers"""
import sqlalchemy
import sqlmodel
from alembic import op as alembic_op
from sqlalchemy import Column as Col, String

from migrations.shared import flags
from migrations.shared.utils import create_index as make_index

revision = "c8"
down_revision = "c7"
branch_labels = None

OFF = False
ON = True


def upgrade():
    alembic_op.create_index("ix_wallets_owner", "wallets", ["owner"])  # caught
    alembic_op.create_index("ix_badges_owner", "badges", ["owner"], postgresql_concurrently=OFF)  # caught
    alembic_op.add_column("wallets", Col("iban", String(34), unique=ON))  # caught
    alembic_op.add_column("badges", sqlmodel.Column("slug", sqlmodel.AutoString(), index=True))  # caught
    alembic_op.add_column("badges", sqlalchemy.Column("kind", String(8), index=flags.NO_INDEX))
    make_index("ledger_entries", "ix_ledger_entries_ts", ["ts"])  # caught
    # On: a constant of this module, or one imported from another module of the run.
    with alembic_op.get_context().autocommit_block():
        alembic_op.create_index("ix_wallets_iban", "wallets", ["iban"], postgresql_concurrently=ON)
        alembic_op.create_index("ix_badges_kind", "badges", ["kind"], postgresql_concurrently=flags.CONCURRENT)


def downgrade():
    pass
