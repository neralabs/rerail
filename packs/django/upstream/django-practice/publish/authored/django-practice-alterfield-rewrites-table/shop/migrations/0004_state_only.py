from django.db import migrations, models


class Migration(migrations.Migration):
    dependencies = [("shop", "0003_alter_types")]

    operations = [
        migrations.SeparateDatabaseAndState(
            # Django's model state only: no ALTER TABLE.
            state_operations=[
                migrations.AlterField(
                    model_name="order",
                    name="email",
                    field=models.IntegerField(),
                ),
            ],
            database_operations=[
                migrations.AlterField(
                    model_name="order",
                    name="legacy_id",
                    field=models.BigIntegerField(blank=True, null=True),  # caught
                ),
            ],
        ),
    ]
