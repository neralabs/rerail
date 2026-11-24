from django.db import migrations, models


def fill_order_numbers(apps, schema_editor):
    Order = apps.get_model("shop", "Order")
    for order in Order.objects.all():
        order.number = f"R{order.pk:08d}"
        order.save(update_fields=["number"])


class Migration(migrations.Migration):
    # A squashed migration runs only on a database that applied none of these: a new, empty one.
    replaces = [("shop", "0001_initial"), ("shop", "0002_order_number")]

    dependencies = []

    operations = [
        migrations.AddField(model_name="customer", name="tier", field=models.CharField(choices=[("basic", "Basic")], max_length=10)),
        migrations.AddField(model_name="order", name="number", field=models.CharField(max_length=20, null=True)),
        migrations.RunPython(fill_order_numbers, migrations.RunPython.noop),
        migrations.AlterField(model_name="order", name="number", field=models.CharField(max_length=20, unique=True)),
    ]
