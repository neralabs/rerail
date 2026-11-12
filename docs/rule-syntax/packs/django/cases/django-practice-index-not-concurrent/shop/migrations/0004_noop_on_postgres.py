from django.db import connection, migrations, models


class Migration(migrations.Migration):
    """The index was built concurrently by hand; this migration only records the state on PostgreSQL."""

    dependencies = [("shop", "0003_concurrent")]

    operations = [
        migrations.AddIndex(
            model_name="book",
            index=models.Index(fields=["sku"], name="book_sku_idx"),
        ),
    ]

    def apply(self, project_state, schema_editor, collect_sql=False):
        if connection.vendor == "postgresql":
            return project_state
        return super().apply(project_state, schema_editor, collect_sql)
