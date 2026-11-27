from django.db import migrations, models
from django.db.models import Count, Min


def remove_duplicate_tags(apps, schema_editor):
    Tag = apps.get_model("shop", "Tag")
    dupes = Tag.objects.values("order", "name").annotate(n=Count("id"), keep=Min("id")).filter(n__gt=1)
    for row in dupes:
        Tag.objects.filter(order=row["order"], name=row["name"]).exclude(pk=row["keep"]).delete()


def add_walk_in_customer(apps, schema_editor):
    Customer = apps.get_model("shop", "Customer")
    Customer.objects.get_or_create(email="walk-in@example.com", defaults={"name": "Walk-in"})


def clear_notes(apps, schema_editor):
    Order = apps.get_model("shop", "Order")
    Order.objects.filter(note="n/a").update(note="")


def zero_prices(apps, schema_editor):
    Product = apps.get_model("shop", "Product")
    Product.objects.filter(price_cents=None).update(price_cents=0)


class Migration(migrations.Migration):
    dependencies = [("shop", "0012_dynamic_model")]

    operations = [
        # No foreign key points at Tag: deleting tags queues no check.
        migrations.RunPython(remove_duplicate_tags, migrations.RunPython.noop),
        migrations.AlterUniqueTogether(name="tag", unique_together={("order", "name")}),
        # Customer has no foreign key of its own: inserting one queues no check.
        migrations.RunPython(add_walk_in_customer, migrations.RunPython.noop),
        migrations.AlterField(model_name="customer", name="name", field=models.CharField(max_length=300)),
        # An update that leaves Order's foreign key as it is queues no check.
        migrations.RunPython(clear_notes, migrations.RunPython.noop),
        migrations.AlterField(model_name="order", name="note", field=models.TextField(blank=True, default="")),
        migrations.AlterField(model_name="order", name="note", field=models.CharField(max_length=500, default="")),
        # Only help_text and default change: Django runs no SQL for this AlterField.
        migrations.AlterField(
            model_name="product",
            name="price_cents",
            field=models.IntegerField(null=True, default=0, help_text="Price in cents"),
        ),
        migrations.RunPython(zero_prices, migrations.RunPython.noop),
    ]
