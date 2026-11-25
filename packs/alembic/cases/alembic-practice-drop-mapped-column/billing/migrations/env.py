from alembic import context

import billing.models as billing_models

context.configure(url="postgresql://billing", target_metadata=billing_models.Base.metadata)
