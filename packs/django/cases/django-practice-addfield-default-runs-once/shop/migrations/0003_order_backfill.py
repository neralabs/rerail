import uuid

from django.db import migrations, models


def fill_numbers(apps, schema_editor):
    Order = apps.get_model("shop", "Order")
    for order in Order.objects.all():
        order.number = uuid.uuid4()
        order.save(update_fields=["number"])
    for order in Order.objects.all():
        order.ref = uuid.uuid4()
        order.save(update_fields=["ref"])


class Migration(migrations.Migration):
    dependencies = [("shop", "0002_order_tokens")]

    operations = [
        # Backfilled per row below, in this migration.
        migrations.AddField(
            model_name="order",
            name="number",
            field=models.UUIDField(default=uuid.uuid4, editable=False),
        ),
        # Unique already: the AddField fails before the backfill runs.
        migrations.AddField(
            model_name="order",
            name="ref",
            field=models.UUIDField(default=uuid.uuid4, unique=True),  # caught
        ),
        migrations.RunPython(fill_numbers, migrations.RunPython.noop),
    ]
