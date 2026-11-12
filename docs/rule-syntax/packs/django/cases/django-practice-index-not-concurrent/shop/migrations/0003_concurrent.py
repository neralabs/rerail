from django.contrib.postgres.operations import AddIndexConcurrently
from django.db import migrations, models


class Migration(migrations.Migration):
    atomic = False

    dependencies = [("shop", "0002_book_indexes")]

    operations = [
        AddIndexConcurrently(
            model_name="book",
            index=models.Index(fields=["title", "subtitle"], name="book_title_sub_idx"),
        ),
        migrations.SeparateDatabaseAndState(
            database_operations=[
                migrations.AddIndex(  # caught
                    model_name="author",
                    index=models.Index(fields=["id", "name"], name="author_id_name_idx"),
                ),
            ],
        ),
    ]
