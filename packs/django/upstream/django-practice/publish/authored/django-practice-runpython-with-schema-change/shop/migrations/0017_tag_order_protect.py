from django.db import migrations, models


def reassign_orphan_tags(apps, schema_editor):
    Order = apps.get_model("shop", "Order")
    Tag = apps.get_model("shop", "Tag")
    fallback = Order.objects.order_by("pk").first()
    for tag in Tag.objects.filter(order__customer__isnull=True):
        tag.order = fallback
        tag.save()


class Migration(migrations.Migration):
    dependencies = [("shop", "0016_sitesettings")]

    # Altering tag.order itself (now nullable) first runs that constraint's queued checks (SET CONSTRAINTS ... IMMEDIATE).
    operations = [
        migrations.RunPython(reassign_orphan_tags, migrations.RunPython.noop),
        migrations.AlterField(
            model_name="tag",
            name="order",
            field=models.ForeignKey(null=True, on_delete=models.PROTECT, to="shop.order"),
        ),
    ]
