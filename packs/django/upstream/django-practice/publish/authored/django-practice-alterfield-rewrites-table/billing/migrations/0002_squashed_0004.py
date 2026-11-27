from django.db import migrations, models


class Migration(migrations.Migration):
    # A squashed migration: its operations already ran wherever the migrations it replaces did.
    replaces = [("billing", "0002_order_amount"), ("billing", "0003_order_note"), ("billing", "0004_order_note")]

    dependencies = [("billing", "0001_initial")]

    operations = [
        migrations.AlterField(
            model_name="order",
            name="note",
            field=models.CharField(max_length=10),
        ),
    ]
