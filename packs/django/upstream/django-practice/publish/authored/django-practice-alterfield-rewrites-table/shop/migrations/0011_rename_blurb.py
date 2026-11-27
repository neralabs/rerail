from django.db import migrations, models


class Migration(migrations.Migration):
    dependencies = [("shop", "0010_custom_fields")]

    operations = [
        migrations.RenameField(model_name="order", old_name="blurb", new_name="summary"),
        # `summary` is the renamed `blurb`, an unlimited varchar since 0003: a limit rewrites the table.
        migrations.AlterField(
            model_name="order",
            name="summary",
            field=models.CharField(max_length=50),  # caught
        ),
    ]
