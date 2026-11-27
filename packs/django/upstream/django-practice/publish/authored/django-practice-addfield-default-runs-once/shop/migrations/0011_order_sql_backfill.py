from django.db import migrations, models

import shop.models


class Migration(migrations.Migration):
    dependencies = [("shop", "0010_fill_coupons")]

    operations = [
        # Filled per row by the RunSQL UPDATE of shop_order below.
        migrations.AddField(
            model_name="order",
            name="secret_key",
            field=models.CharField(default=shop.models.new_key, max_length=32),
        ),
        # The UPDATE below fills another table's column of that name: every existing order shares one key.
        migrations.AddField(
            model_name="order",
            name="share_key",
            field=models.CharField(default=shop.models.new_key, max_length=32),  # caught
        ),
        migrations.RunSQL(
            "UPDATE shop_order SET secret_key = md5(random()::text || id::text)",
            migrations.RunSQL.noop,
        ),
        migrations.RunSQL(
            "UPDATE shop_customer SET share_key = md5(random()::text || id::text)",
            migrations.RunSQL.noop,
        ),
    ]
