import functools
import random
import uuid
from secrets import token_hex
from uuid import uuid4 as make_id

from django.db import migrations, models

import shop.models


class Migration(migrations.Migration):
    dependencies = [("shop", "0012_more_generators")]

    operations = [
        # Generators under an import alias, by from-import, and through random.SystemRandom.
        migrations.AddField(
            model_name="order",
            name="alias_id",
            field=models.UUIDField(default=make_id),  # caught
        ),
        migrations.AddField(
            model_name="order",
            name="hex_token",
            field=models.CharField(default=token_hex, max_length=64),  # caught
        ),
        migrations.AddField(
            model_name="order",
            name="pin_code",
            field=models.CharField(default=shop.models.system_pin, max_length=6),  # caught
        ),
        # The random module's shared generator, two hops deep or through functools.partial: cosmetic, left alone.
        migrations.AddField(
            model_name="profile",
            name="colour_name",
            field=models.CharField(default=shop.models.colour_label, max_length=20),
        ),
        migrations.AddField(
            model_name="profile",
            name="lucky_number",
            field=models.IntegerField(default=functools.partial(random.randint, 0, 9)),
        ),
        # Backfilled by 0014, each in another shape.
        migrations.AddField(
            model_name="order",
            name="ext_ref",
            field=models.UUIDField(default=uuid.uuid4, null=True),
        ),
        migrations.AddField(
            model_name="customer",
            name="invite_code",
            field=models.CharField(default=shop.models.new_key, max_length=32),
        ),
        migrations.AddField(
            model_name="order",
            name="batch_token",
            field=models.CharField(default=shop.models.new_key, max_length=32),
        ),
        # 0014's UPDATE sets a column of that name on shop_customer, not shop_order.
        migrations.AddField(
            model_name="order",
            name="batch_key",
            field=models.CharField(default=shop.models.new_key, max_length=32),  # caught
        ),
    ]
