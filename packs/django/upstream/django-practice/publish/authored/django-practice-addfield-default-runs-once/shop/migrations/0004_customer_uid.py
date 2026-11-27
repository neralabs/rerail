import secrets
import uuid

from django.db import migrations, models


class Migration(migrations.Migration):
    dependencies = [("shop", "0003_order_backfill")]

    # Step 1 of 3: added without unique; 0005 gives each row its own value, 0006 makes it unique.
    operations = [
        migrations.AddField(
            model_name="customer",
            name="uid",
            field=models.UUIDField(default=uuid.uuid4),
        ),
        migrations.AddField(
            model_name="customer",
            name="api_key",
            field=models.CharField(default=secrets.token_hex, max_length=64),
        ),
    ]
