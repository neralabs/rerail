import uuid

from django.db import migrations, models


class Migration(migrations.Migration):
    dependencies = [("shop", "0005_fill_customer_uid")]

    operations = [
        migrations.AlterField(
            model_name="customer",
            name="uid",
            field=models.UUIDField(default=uuid.uuid4, unique=True),
        ),
    ]
