from django.db import migrations, models


class Migration(migrations.Migration):
    dependencies = [("shop", "0001_initial")]

    operations = [
        migrations.AlterField(
            model_name="rating",
            name="score",
            field=models.BigIntegerField(),
        ),
    ]
