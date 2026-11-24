import uuid

from django.db import migrations, models


class Migration(migrations.Migration):
    dependencies = [("shop", "0007_profile_fields")]

    operations = [
        # SiteConfig has one row (shop/views.py reads it with objects.get()): one value is right.
        migrations.AddField(
            model_name="siteconfig",
            name="install_id",
            field=models.UUIDField(default=uuid.uuid4, editable=False),
        ),
        migrations.AddField(
            model_name="order",
            name="tracking_id",
            field=models.UUIDField(default=uuid.uuid4, editable=False),  # caught
        ),
    ]
