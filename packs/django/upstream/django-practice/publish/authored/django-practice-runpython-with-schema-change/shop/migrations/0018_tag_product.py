from django.db import migrations, models


def drop_unpriced_products(apps, schema_editor):
    Product = apps.get_model("shop", "Product")
    Product.objects.filter(price_cents=None).delete()


class Migration(migrations.Migration):
    dependencies = [("shop", "0017_tag_order_protect")]

    # No foreign key points at Product yet when its rows are deleted: tag.product comes after.
    operations = [
        migrations.RunPython(drop_unpriced_products, migrations.RunPython.noop),
        migrations.AddField(
            model_name="tag",
            name="product",
            field=models.ForeignKey(null=True, on_delete=models.SET_NULL, to="shop.product"),
        ),
        migrations.AlterField(model_name="product", name="price_cents", field=models.IntegerField()),
    ]
