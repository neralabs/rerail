from alembic import context

from ledger.registry import metadata

context.configure(url="postgresql://ledger", target_metadata=metadata)
