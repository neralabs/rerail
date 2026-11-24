from django.db import migrations, models
from django.db.models import BigAutoField


class Migration(migrations.Migration):
    dependencies = [("shop", "0002_order_legacy_id")]

    operations = [
        migrations.AlterField(
            model_name="order",
            name="id",
            field=models.BigAutoField(auto_created=True, primary_key=True, serialize=False),  # caught
        ),
        migrations.AlterField(
            model_name="customer",
            name="id",
            field=BigAutoField(auto_created=True, primary_key=True, serialize=False),  # caught
        ),
        migrations.AlterField(
            model_name="tag",
            name="id",
            field=models.AutoField(auto_created=True, primary_key=True, serialize=False),  # caught
        ),
        migrations.AlterField("order", "qty", models.BigIntegerField()),  # caught
        migrations.AlterField(
            model_name="order",
            name="small",
            field=models.IntegerField(),  # caught
        ),
        migrations.AlterField(
            model_name="order",
            name="views",
            field=models.PositiveBigIntegerField(default=0),  # caught
        ),
        migrations.AlterField(
            model_name="order",
            name="number",
            field=models.CharField(max_length=10),  # caught
        ),
        # The nearest earlier definition is 0002's max_length=200, not 0001's 100: this shrinks.
        migrations.AlterField(
            model_name="order",
            name="note",
            field=models.CharField(max_length=150),  # caught
        ),
        migrations.AlterField(
            model_name="order",
            name="body",
            field=models.CharField(max_length=500),  # caught
        ),
        migrations.AlterField(
            model_name="order",
            name="weight",
            field=models.DecimalField(decimal_places=3, max_digits=10),  # caught
        ),
        migrations.AlterField(
            model_name="order",
            name="code",
            field=models.IntegerField(),  # caught
        ),
        migrations.AlterField(
            model_name="order",
            name="seats",
            field=models.CharField(max_length=10),  # caught
        ),
        migrations.AlterField(
            model_name="order",
            name="ratio",
            field=models.DecimalField(decimal_places=2, max_digits=5),  # caught
        ),
        migrations.AlterField(
            model_name="order",
            name="placed",
            field=models.DateTimeField(),  # caught
        ),
        migrations.AlterField(
            model_name="order",
            name="meta",
            field=models.JSONField(default=dict),  # caught
        ),
        migrations.AlterField(
            model_name="order",
            name="site2",
            field=models.CharField(max_length=100),  # caught
        ),
        # Left alone: no rewrite.
        migrations.AlterField(
            model_name="order",
            name="total",
            field=models.DecimalField(decimal_places=2, max_digits=12),
        ),
        migrations.AlterField(
            model_name="order",
            name="ref",
            field=models.TextField(),
        ),
        migrations.AlterField(
            model_name="order",
            name="slug",
            field=models.SlugField(max_length=100),
        ),
        migrations.AlterField(
            model_name="order",
            name="handle",
            field=models.SlugField(),
        ),
        migrations.AlterField(
            model_name="order",
            name="site",
            field=models.URLField(),
        ),
        migrations.AlterField(
            model_name="order",
            name="email",
            field=models.EmailField(),
        ),
        migrations.AlterField(
            model_name="order",
            name="blurb",
            field=models.CharField(max_length=None),
        ),
        migrations.AlterField(
            model_name="order",
            name="flag",
            field=models.BooleanField(default=True),
        ),
        migrations.AlterField(
            model_name="order",
            name="legacy_id",
            field=models.IntegerField(blank=True, null=True, db_index=True),
        ),
        migrations.AlterField(
            model_name="order",
            name="customer",
            field=models.ForeignKey(null=True, on_delete=models.SET_NULL, to="shop.customer"),
        ),
        # The rule cannot tell these types: a length that is no literal, a third-party field class.
        migrations.AlterField(
            model_name="order",
            name="tracking",
            field=models.CharField(max_length=8),
        ),
        migrations.AlterField(
            model_name="order",
            name="country",
            field=models.CharField(max_length=2),
        ),
    ]
