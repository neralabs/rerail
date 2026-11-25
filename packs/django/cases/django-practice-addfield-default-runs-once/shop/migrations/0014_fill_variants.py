import secrets
import uuid

from django.db import migrations

from shop.models import Customer as Client


def fill_ext_refs(apps, schema_editor):
    OrderModel = apps.get_model(app_label="shop", model_name="Order")
    for o in OrderModel.objects.iterator():
        setattr(o, "ext_ref", uuid.uuid4())
        o.save()


def fill_invite_codes(apps, schema_editor):
    for pk in Client._default_manager.values_list("pk", flat=True):
        Client._default_manager.filter(pk=pk).update(invite_code=secrets.token_hex(16))


class Migration(migrations.Migration):
    dependencies = [("shop", "0013_generator_variants")]

    operations = [
        migrations.RunPython(fill_ext_refs, migrations.RunPython.noop),
        migrations.RunPython(fill_invite_codes, migrations.RunPython.noop),
        migrations.RunSQL(
            ['UPDATE "shop_order" SET "batch_token" = md5(random()::text || id::text)'],
            migrations.RunSQL.noop,
        ),
        migrations.RunSQL(
            [("UPDATE shop_customer SET batch_key = md5(random()::text || %s)", ["salt"])],
            migrations.RunSQL.noop,
        ),
    ]
