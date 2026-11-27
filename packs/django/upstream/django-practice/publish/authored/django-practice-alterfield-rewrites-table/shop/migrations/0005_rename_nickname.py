from django.db import migrations, models


class Migration(migrations.Migration):
    dependencies = [("shop", "0004_state_only")]

    operations = [
        migrations.RemoveField(model_name="order", name="note"),
        migrations.RenameField(model_name="order", old_name="nickname", new_name="note"),
        migrations.RemoveField(model_name="order", name="code"),
        migrations.AddField(
            model_name="order",
            name="code",
            field=models.CharField(max_length=10, null=True),
        ),
    ]
