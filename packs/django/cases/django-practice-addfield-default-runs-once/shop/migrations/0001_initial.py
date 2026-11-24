import uuid

from django.db import migrations, models

import shop.models


class Migration(migrations.Migration):
    initial = True

    dependencies = []

    operations = [
        migrations.CreateModel(
            name="Order",
            fields=[
                ("id", models.AutoField(primary_key=True, serialize=False)),
                ("token", models.UUIDField(default=uuid.uuid4, unique=True)),
            ],
        ),
        migrations.CreateModel(
            name="Customer",
            fields=[
                ("id", models.AutoField(primary_key=True, serialize=False)),
                ("name", models.CharField(max_length=100)),
            ],
        ),
        migrations.CreateModel(
            name="Profile",
            fields=[("id", models.AutoField(primary_key=True, serialize=False))],
        ),
        # The table is created above, in this migration: it has no rows yet.
        migrations.AddField(
            model_name="order",
            name="secret",
            field=models.CharField(default=shop.models.generate_secret, max_length=16, unique=True),
        ),
    ]
