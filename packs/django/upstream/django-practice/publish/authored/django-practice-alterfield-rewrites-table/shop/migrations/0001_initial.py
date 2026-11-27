import django.db.models.deletion
import django_countries.fields
from django.conf import settings
from django.db import migrations, models


class Migration(migrations.Migration):
    initial = True

    dependencies = []

    operations = [
        migrations.CreateModel(
            name="Customer",
            fields=[
                ("id", models.AutoField(auto_created=True, primary_key=True, serialize=False)),
                ("name", models.CharField(max_length=100)),
            ],
        ),
        migrations.CreateModel(
            name="Tag",
            fields=[
                ("id", models.SmallAutoField(auto_created=True, primary_key=True, serialize=False)),
            ],
        ),
        migrations.CreateModel(
            name="Order",
            fields=[
                ("id", models.AutoField(auto_created=True, primary_key=True, serialize=False)),
                ("number", models.CharField(max_length=20)),
                ("note", models.CharField(max_length=100)),
                ("nickname", models.CharField(max_length=30)),
                ("body", models.TextField()),
                ("total", models.DecimalField(decimal_places=2, max_digits=10)),
                ("weight", models.DecimalField(decimal_places=2, max_digits=8)),
                ("qty", models.IntegerField()),
                ("small", models.SmallIntegerField()),
                ("views", models.PositiveIntegerField(default=0)),
                ("seats", models.IntegerField()),
                ("ratio", models.FloatField()),
                ("code", models.CharField(max_length=10)),
                ("slug", models.SlugField()),
                ("handle", models.CharField(max_length=50)),
                ("site", models.CharField(max_length=200)),
                ("site2", models.URLField()),
                ("email", models.CharField(max_length=254)),
                ("ref", models.CharField(max_length=40)),
                ("blurb", models.CharField(max_length=100)),
                ("placed", models.DateField()),
                ("meta", models.TextField()),
                ("flag", models.BooleanField()),
                ("amount", models.IntegerField()),
                ("tracking", models.CharField(max_length=settings.TRACKING_LENGTH)),
                ("country", django_countries.fields.CountryField(max_length=2)),
                (
                    "customer",
                    models.ForeignKey(on_delete=django.db.models.deletion.CASCADE, to="shop.customer"),
                ),
            ],
        ),
        migrations.CreateModel(
            name="Draft",
            fields=[
                ("id", models.AutoField(auto_created=True, primary_key=True, serialize=False)),
                ("title", models.CharField(max_length=200)),
            ],
        ),
        # The table is created above, in this migration: it has no rows to rewrite.
        migrations.AlterField(
            model_name="draft",
            name="title",
            field=models.CharField(max_length=20),
        ),
    ]
