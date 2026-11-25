import uuid

from django.db import migrations, models

import shop.models


class Migration(migrations.Migration):
    dependencies = [("shop", "0006_customer_uid_unique")]

    operations = [
        migrations.SeparateDatabaseAndState(
            database_operations=[
                migrations.AddField(
                    model_name="profile",
                    name="handle",
                    field=models.UUIDField(default=uuid.uuid4),  # caught
                ),
            ],
            state_operations=[
                migrations.AddField(
                    model_name="profile",
                    name="legacy_handle",
                    field=models.UUIDField(default=uuid.uuid4),
                ),
            ],
        ),
        # A random colour per profile: every existing profile gets the same one, which is cosmetic.
        migrations.AddField(
            model_name="profile",
            name="background_color",
            field=models.CharField(default=shop.models.pick_colour, max_length=7),
        ),
    ]
