from django.db import migrations, models

import shop.fields


class Migration(migrations.Migration):
    dependencies = [("shop", "0009_order_refs")]

    operations = [
        migrations.AddField(
            model_name="order",
            name="price",
            field=shop.fields.MoneyField(decimal_places=2, default=0, max_digits=10),
        ),
        migrations.AddField(model_name="order", name="label", field=shop.fields.LabelField(max_length=50, default="")),
        migrations.AddField(model_name="order", name="spot", field=models.IntegerField(null=True)),
        # MoneyField is a DecimalField: a new scale rewrites the table.
        migrations.AlterField(
            model_name="order",
            name="price",
            field=shop.fields.MoneyField(decimal_places=3, default=0, max_digits=10),  # caught
        ),
        # LabelField's __init__ can rewrite max_length (InvenTree's URL field forces 2000): a shorter one is not judged.
        migrations.AlterField(model_name="order", name="label", field=shop.fields.LabelField(max_length=20, default="")),
        # GeoPointField sets its own db_type: not judged.
        migrations.AlterField(model_name="order", name="spot", field=shop.fields.GeoPointField(null=True)),
    ]
