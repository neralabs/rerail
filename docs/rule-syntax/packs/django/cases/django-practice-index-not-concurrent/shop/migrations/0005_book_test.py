from django.db import migrations, models


class Migration(migrations.Migration):
    """A migration whose name ends in `_test` is still a migration, not test code."""

    dependencies = [("shop", "0004_noop_on_postgres")]

    operations = [
        migrations.AddField(  # caught
            model_name="author",
            name="test",
            field=models.ForeignKey(null=True, on_delete=models.SET_NULL, to="shop.book"),
        ),
    ]
