import uuid

from django.db import migrations, models

import shop.models


class Migration(migrations.Migration):
    dependencies = [("shop", "0008_siteconfig_install_id")]

    operations = [
        # 0010 fills `coupon` per row for customers only: every existing order still shares one key.
        migrations.AddField(
            model_name="order",
            name="coupon",
            field=models.CharField(default=shop.models.new_key, max_length=32),  # caught
        ),
        # Filled per row by 0010's fill_customer_coupons (apps.get_model("shop", "Customer")).
        migrations.AddField(
            model_name="customer",
            name="coupon",
            field=models.CharField(default=shop.models.new_key, max_length=32),
        ),
        # Filled per row by 0010's fill_order_vouchers, through a helper (apps.get_model("shop.Order")).
        migrations.AddField(
            model_name="order",
            name="voucher",
            field=models.UUIDField(default=uuid.uuid4, null=True),
        ),
    ]
