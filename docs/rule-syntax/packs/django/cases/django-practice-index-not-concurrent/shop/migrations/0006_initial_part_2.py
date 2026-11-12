from django.db import migrations, models


class Migration(migrations.Migration):
    """makemigrations split the app's first migration: these tables are new, so their indexes are free."""

    initial = True

    dependencies = [("shop", "0005_book_test")]

    operations = [
        migrations.AddField(
            model_name="book",
            name="coauthor",
            field=models.ForeignKey(null=True, on_delete=models.SET_NULL, related_name="+", to="shop.author"),
        ),
    ]
