import uuid

from django.db import migrations, models


class Migration(migrations.Migration):
    dependencies = [("billing", "0001_initial")]

    operations = [
        migrations.AddField(
            model_name="invoice",
            name="secret",
            field=models.UUIDField(default=uuid.uuid4),  # caught
        ),
    ]
