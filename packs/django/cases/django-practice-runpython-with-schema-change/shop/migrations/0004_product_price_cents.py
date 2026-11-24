from django.db import migrations, models

from shop.backfills import copy_prices


class Migration(migrations.Migration):
    dependencies = [("shop", "0003_merge_customers")]

    # The backfill function lives in shop/backfills.py.
    operations = [
        migrations.AddField(
            model_name="product",
            name="price_cents",
            field=models.IntegerField(null=True),
        ),
        migrations.RunPython(copy_prices, migrations.RunPython.noop),  # caught
        migrations.RemoveField(model_name="product", name="price"),
    ]
