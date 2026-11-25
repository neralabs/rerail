import random
import secrets

import shortuuid
from django.db import migrations, models

import shop.models


class Migration(migrations.Migration):
    dependencies = [("shop", "0011_order_sql_backfill")]

    operations = [
        migrations.AddField(
            model_name="order",
            name="short",
            field=models.CharField(default=shortuuid.uuid, max_length=22),  # caught
        ),
        # Nullable is no exemption: existing rows get the one default, not NULL.
        migrations.AddField(
            model_name="customer",
            name="reset_token",
            field=models.CharField(default=secrets.token_urlsafe, max_length=64, null=True),  # caught
        ),
        migrations.AddField(
            model_name="order",
            name="handle",
            field=models.SlugField(default="", unique=True),  # caught
            preserve_default=False,
        ),
        # random.* is not a secret: one tagline or hue for existing rows is cosmetic.
        migrations.AddField(
            model_name="profile",
            name="tagline",
            field=models.CharField(default=shop.models.pick_tagline, max_length=40),
        ),
        migrations.AddField(
            model_name="profile",
            name="hue",
            field=models.FloatField(default=random.random),
        ),
    ]
