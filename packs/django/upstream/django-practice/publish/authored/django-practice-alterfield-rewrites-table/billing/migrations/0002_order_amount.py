from django.db import migrations, models


class Migration(migrations.Migration):
    dependencies = [("billing", "0001_initial"), ("shop", "0006_widen")]

    operations = [
        # billing's Order, not shop's: its amount is a bigint already.
        migrations.AlterField(
            model_name="order",
            name="amount",
            field=models.BigIntegerField(null=True),
        ),
        migrations.AlterField(
            model_name="order",
            name="note",
            field=models.CharField(max_length=300),  # caught
        ),
        # No earlier definition in this app's history: left alone.
        migrations.AlterField(
            model_name="invoice",
            name="id",
            field=models.BigAutoField(auto_created=True, primary_key=True, serialize=False),
        ),
    ]
