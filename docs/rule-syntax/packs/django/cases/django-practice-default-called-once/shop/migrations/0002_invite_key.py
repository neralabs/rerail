import uuid

from django.db import migrations, models


class Migration(migrations.Migration):
    dependencies = [("shop", "0001_initial")]

    # A one-off default for existing rows: computed once on purpose.
    operations = [
        migrations.AddField(
            model_name="invite",
            name="other_key",
            field=models.UUIDField(default=uuid.uuid4()),
            preserve_default=False,
        ),
    ]
