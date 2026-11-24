from django.db import migrations, models


class Migration(migrations.Migration):
    dependencies = [("shop", "0001_initial")]

    operations = [
        migrations.AddField(
            model_name="order",
            name="legacy_id",
            field=models.IntegerField(null=True),
        ),
        # Widening a varchar changes the catalog only.
        migrations.AlterField(
            model_name="order",
            name="note",
            field=models.CharField(max_length=200),
        ),
    ]
