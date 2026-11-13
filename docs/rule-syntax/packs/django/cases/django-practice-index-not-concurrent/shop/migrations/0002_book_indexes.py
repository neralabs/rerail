from django.db import migrations, models
from django.db.migrations import AddField


class Migration(migrations.Migration):
    dependencies = [("shop", "0001_initial")]

    operations = [
        migrations.AddIndex(  # caught
            model_name="book",
            index=models.Index(fields=["subtitle"], name="book_subtitle_idx"),
        ),
        migrations.AddField(  # caught
            model_name="book",
            name="author",
            field=models.ForeignKey(null=True, on_delete=models.CASCADE, to="shop.author"),
        ),
        AddField("book", "isbn", models.CharField(max_length=13, null=True, unique=True)),  # caught
        migrations.AddField(  # caught
            model_name="book",
            name="sku",
            field=models.CharField(max_length=20, default="", db_index=True),
        ),
        migrations.AlterField(  # caught
            model_name="book",
            name="subtitle",
            field=models.CharField(max_length=200, db_index=True),
        ),
        # Near misses: no index, or the index already exists.
        migrations.AddField(
            model_name="book",
            name="blurb",
            field=models.TextField(default=""),
        ),
        migrations.AddField(
            model_name="book",
            name="editor",
            field=models.ForeignKey(null=True, db_index=False, on_delete=models.SET_NULL, to="shop.author"),
        ),
        migrations.AddField(
            model_name="book",
            name="tags",
            field=models.ManyToManyField(to="shop.author"),
        ),
        migrations.AlterField(
            model_name="book",
            name="title",
            field=models.CharField(max_length=200, db_index=True),
        ),
        migrations.AlterField(
            model_name="book",
            name="code",
            field=models.CharField(max_length=20, unique=True),
        ),
        # Added with its index earlier in this migration: altering its max_length builds nothing more.
        migrations.AlterField(
            model_name="book",
            name="sku",
            field=models.CharField(max_length=40, default="", db_index=True),
        ),
        # Renamed: the column keeps its index; no earlier definition of "writer" says so, and the rule stays silent.
        migrations.RenameField(model_name="book", old_name="author", new_name="writer"),
        migrations.AlterField(
            model_name="book",
            name="writer",
            field=models.ForeignKey(null=True, on_delete=models.SET_NULL, to="shop.author"),
        ),
        # Only the state changes; the database operation builds the index concurrently.
        migrations.SeparateDatabaseAndState(
            state_operations=[
                migrations.AddIndex(
                    model_name="book",
                    index=models.Index(fields=["blurb"], name="book_blurb_idx"),
                ),
            ],
            database_operations=[
                migrations.RunSQL("CREATE INDEX CONCURRENTLY book_blurb_idx ON shop_book (blurb)"),
            ],
        ),
        # A new table in this migration, and a model no migration here created.
        migrations.CreateModel(
            name="Shelf",
            fields=[("id", models.AutoField(primary_key=True)), ("label", models.CharField(max_length=10))],
        ),
        migrations.AddIndex(
            model_name="shelf",
            index=models.Index(fields=["label"], name="shelf_label_idx"),
        ),
        # An unmanaged model: Django builds nothing for it.
        migrations.AddIndex(
            model_name="salesview",
            index=models.Index(fields=["total"], name="sales_total_idx"),
        ),
        migrations.AddIndex(
            model_name="ghost",
            index=models.Index(fields=["x"], name="ghost_x_idx"),
        ),
    ]
