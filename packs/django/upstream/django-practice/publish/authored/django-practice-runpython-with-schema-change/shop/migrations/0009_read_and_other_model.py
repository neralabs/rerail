import logging

from django.db import migrations, models

logger = logging.getLogger(__name__)


def count_orders(apps, schema_editor):
    Order = apps.get_model("shop", "Order")
    for order in Order.objects.all():
        logger.info("order %s", order.pk)
    logger.info("%d orders", Order.objects.count())


def touch_customers(apps, schema_editor):
    Customer = apps.get_model("shop", "Customer")
    Customer.objects.update(name="x")


def bump_options(apps, schema_editor):
    Product = apps.get_model("shop", "Product")
    Product.objects.update(price_cents=0)


class Migration(migrations.Migration):
    dependencies = [("shop", "0008_nonatomic")]

    operations = [
        # Reads Order only.
        migrations.RunPython(count_orders, migrations.RunPython.noop),
        # Writes Customer; the schema change below is Order's.
        migrations.RunPython(touch_customers, migrations.RunPython.noop),
        migrations.AddField(model_name="order", name="note", field=models.TextField(default="")),
        # Writes Product; AlterModelOptions changes no table.
        migrations.RunPython(bump_options, migrations.RunPython.noop),
        migrations.AlterModelOptions(name="product", options={"ordering": ["pk"]}),
    ]
