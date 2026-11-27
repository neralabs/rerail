import secrets
import uuid

from django.db import migrations


def fill_customer_coupons(apps, schema_editor):
    Customer = apps.get_model("shop", "Customer")
    for customer in Customer.objects.all():
        customer.coupon = secrets.token_hex(16)
        customer.save(update_fields=["coupon"])


def _fill_vouchers(model):
    for order in model.objects.all():
        order.voucher = uuid.uuid4()
        order.save(update_fields=["voucher"])


def fill_order_vouchers(apps, schema_editor):
    _fill_vouchers(apps.get_model("shop.Order"))


class Migration(migrations.Migration):
    dependencies = [("shop", "0009_coupons")]

    operations = [
        migrations.RunPython(fill_customer_coupons, migrations.RunPython.noop),
        migrations.RunPython(fill_order_vouchers, migrations.RunPython.noop),
    ]
