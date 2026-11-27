import secrets
import uuid

from django.db import migrations


def new_uid():
    return uuid.uuid4()


def fill_uids(apps, schema_editor):
    Customer = apps.get_model("shop", "Customer")
    for pk in Customer.objects.values_list("pk", flat=True):
        Customer.objects.filter(pk=pk).update(uid=new_uid(), api_key=secrets.token_hex())


class Migration(migrations.Migration):
    dependencies = [("shop", "0004_customer_uid")]

    operations = [migrations.RunPython(fill_uids, migrations.RunPython.noop)]
