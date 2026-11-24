from django.db import migrations, models


class Migration(migrations.Migration):
    # 0004_order_note was squashed: Django reads this dependency as 0002_squashed_0004, which replaces it.
    dependencies = [("billing", "0004_order_note")]

    operations = [
        migrations.AlterField(
            model_name="order",
            name="amount",
            field=models.IntegerField(),  # caught
        ),
        # The squashed migration made `note` a varchar(10): 20 widens it.
        migrations.AlterField(
            model_name="order",
            name="note",
            field=models.CharField(max_length=20),
        ),
    ]
