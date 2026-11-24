from django.db import migrations, models


class Migration(migrations.Migration):
    dependencies = [("shop", "0005_rename_nickname")]

    operations = [
        # `note` is the renamed `nickname` (max_length=30) now, so 100 widens it.
        migrations.AlterField(
            model_name="order",
            name="note",
            field=models.CharField(max_length=100),
        ),
        # `code` was re-added in 0005 as a varchar(10); 0003's IntegerField is no longer its type.
        migrations.AlterField(
            model_name="order",
            name="code",
            field=models.CharField(max_length=20, null=True),
        ),
        migrations.AlterField(
            model_name="order",
            name="qty",
            field=models.IntegerField(),  # caught
        ),
    ]
