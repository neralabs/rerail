"""Not a revision: no module-level revision/down_revision. A script that patches a scratch database."""
from alembic import op


def add_constraints():
    op.create_foreign_key("fk_orders_user", "orders", "users", ["user_id"], ["id"])


class FakeOps:
    def create_foreign_key(self, *args, **kwargs):
        return args


def upgrade():
    op2 = FakeOps()
    op2.create_foreign_key("fk", "orders", "users", ["user_id"], ["id"])


class ForeignKey:
    """A project's own reference marker that shares SQLAlchemy's class name."""

    def __init__(self, target):
        self.target = target
