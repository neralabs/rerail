import functools
import os
import secrets
import uuid
from datetime import date

import django.utils.timezone
from django.db import migrations, models
from django.db.models.functions import Random
from django.utils.crypto import get_random_string

import shop.models
from shop.models import new_key


def get_token():
    return str(uuid.uuid4())


def legacy_label():
    return "legacy"


class Migration(migrations.Migration):
    dependencies = [("shop", "0001_initial")]

    operations = [
        migrations.AddField(
            model_name="order",
            name="web_secret",
            field=models.CharField(db_index=True, default=shop.models.generate_secret, max_length=32),  # caught
        ),
        migrations.AddField(
            model_name="order",
            name="public_id",
            field=models.UUIDField(default=uuid.uuid4, editable=False),  # caught
        ),
        migrations.AddField(
            model_name="order",
            name="api_token",
            field=models.CharField(default=secrets.token_urlsafe, max_length=64),  # caught
        ),
        migrations.AddField(
            model_name="order",
            name="salt",
            field=models.BinaryField(default=os.urandom),  # caught
        ),
        migrations.AddField(
            model_name="order",
            name="pin",
            field=models.CharField(default=functools.partial(get_random_string, 6), max_length=6),  # caught
        ),
        migrations.AddField(
            "order",
            "code",
            models.CharField(default=shop.models.generate_code, max_length=20),  # caught
        ),
        migrations.AddField(
            model_name="order",
            name="token",
            field=models.UUIDField(default=get_token, editable=False),  # caught
            preserve_default=False,
        ),
        migrations.AddField(
            model_name="customer",
            name="key",
            field=models.CharField(default=new_key, max_length=32),  # caught
        ),
        migrations.AddField(
            model_name="customer",
            name="number",
            field=models.CharField(default="0000", max_length=8, unique=True),  # caught
        ),
        migrations.AddField(
            model_name="customer",
            name="login_id",
            field=models.UUIDField(default=uuid.uuid4, null=True, unique=True),  # caught
        ),
        migrations.AddField(
            model_name="customer",
            name="profile",
            field=models.OneToOneField(default=1, on_delete=models.CASCADE, to="shop.profile"),  # caught
        ),
        migrations.AddField(
            model_name="customer",
            name="ref",
            field=models.UUIDField(default=uuid.uuid4, primary_key=True, serialize=False),  # caught
        ),
        # Near misses: one value for existing rows is what these mean.
        migrations.AddField(
            model_name="order",
            name="created",
            field=models.DateTimeField(default=django.utils.timezone.now),
        ),
        migrations.AddField(
            model_name="order",
            name="day",
            field=models.DateField(default=date.today),
        ),
        migrations.AddField(
            model_name="order",
            name="expires",
            field=models.DateTimeField(default=shop.models.default_expiry),
        ),
        migrations.AddField(
            model_name="order",
            name="prefs",
            field=models.JSONField(default=shop.models.default_prefs),
        ),
        migrations.AddField(
            model_name="order",
            name="tags",
            field=models.JSONField(default=dict),
        ),
        migrations.AddField(
            model_name="order",
            name="label",
            field=models.CharField(default=legacy_label, max_length=20),
        ),
        migrations.AddField(
            model_name="order",
            name="colour",
            field=models.CharField(default="blue", max_length=20),
        ),
        migrations.AddField(
            model_name="order",
            name="once",
            field=models.UUIDField(default=uuid.uuid4()),
        ),
        migrations.AddField(
            model_name="order",
            name="maybe_id",
            field=models.UUIDField(default=uuid.uuid4, null=True),
        ),
        migrations.AddField(
            model_name="order",
            name="db_id",
            field=models.UUIDField(default=uuid.uuid4, db_default=Random()),
        ),
        migrations.AddField(
            model_name="order",
            name="external_id",
            field=models.CharField(default=None, max_length=40, null=True, unique=True),
        ),
        migrations.AddField(
            model_name="order",
            name="slug",
            field=models.SlugField(default="", unique=False),
        ),
    ]
