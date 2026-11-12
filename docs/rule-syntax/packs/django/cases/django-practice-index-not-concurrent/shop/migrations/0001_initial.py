from django.db import migrations, models


class Migration(migrations.Migration):
    initial = True

    dependencies = []

    operations = [
        migrations.CreateModel(
            name="Author",
            fields=[
                ("id", models.AutoField(primary_key=True)),
                ("name", models.CharField(max_length=100)),
            ],
        ),
        migrations.CreateModel(
            name="Book",
            fields=[
                ("id", models.AutoField(primary_key=True)),
                ("title", models.CharField(max_length=100, db_index=True)),
                ("subtitle", models.CharField(max_length=100)),
                ("code", models.CharField(max_length=10, unique=True)),
            ],
        ),
        migrations.CreateModel(
            name="SalesView",
            fields=[("id", models.AutoField(primary_key=True)), ("total", models.IntegerField())],
            options={"managed": False, "db_table": "sales_view"},
        ),
        # The table is created in this migration: it is empty, so a plain index is fine.
        migrations.AddIndex(
            model_name="author",
            index=models.Index(fields=["name"], name="author_name_idx"),
        ),
    ]
