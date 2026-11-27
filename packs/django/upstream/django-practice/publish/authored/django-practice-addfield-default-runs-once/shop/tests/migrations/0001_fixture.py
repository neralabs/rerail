import uuid

from django.db import migrations, models


class Migration(migrations.Migration):
    dependencies = []

    operations = [
        migrations.AddField(
            model_name="widget",
            name="uid",
            field=models.UUIDField(default=uuid.uuid4, unique=True),
        ),
    ]
