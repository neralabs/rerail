import uuid

from django.db import migrations, models


class Migration(migrations.Migration):
    dependencies = [("shop", "0014_fill_variants")]

    operations = [
        # StoreSettings is the current site's settings row (shop/views.py: Site.objects.get_current().store_settings):
        # one installation id for it is right.
        migrations.AddField(
            model_name="storesettings",
            name="instance_id",
            field=models.UUIDField(blank=True, default=uuid.uuid4, null=True),
        ),
        # Profile is read through request.user.profile, one row per user: the table has many rows.
        migrations.AddField(
            model_name="profile",
            name="feed_token",
            field=models.UUIDField(default=uuid.uuid4),  # caught
        ),
    ]
