from django.db import migrations, models


def fill(apps, schema_editor):
    Widget = apps.get_model("tests", "Widget")
    Widget.objects.update(size=1)


class Migration(migrations.Migration):
    dependencies = []

    operations = [
        migrations.RunPython(fill, migrations.RunPython.noop),
        migrations.AlterField(model_name="widget", name="size", field=models.IntegerField()),
    ]
