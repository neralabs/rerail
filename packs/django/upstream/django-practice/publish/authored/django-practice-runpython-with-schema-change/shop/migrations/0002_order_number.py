from django.db import migrations, models


def fill_order_numbers(apps, schema_editor):
    Order = apps.get_model("shop", "Order")
    for order in Order.objects.order_by("pk").iterator():
        order.number = f"R{order.pk:08d}"
        order.save(update_fields=["number"])


class Migration(migrations.Migration):
    dependencies = [("shop", "0001_initial")]

    # Add nullable, backfill, then make it unique: three steps in one transaction.
    operations = [
        migrations.AddField(
            model_name="order",
            name="number",
            field=models.CharField(max_length=20, null=True),
        ),
        migrations.RunPython(fill_order_numbers, migrations.RunPython.noop),  # caught
        migrations.AlterField(
            model_name="order",
            name="number",
            field=models.CharField(max_length=20, unique=True),
        ),
    ]
