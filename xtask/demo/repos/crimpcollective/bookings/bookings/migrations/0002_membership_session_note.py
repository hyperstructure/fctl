import django.db.models.deletion
from django.conf import settings
from django.db import migrations, models


class Migration(migrations.Migration):
    dependencies = [
        migrations.swappable_dependency(settings.AUTH_USER_MODEL),
        ("bookings", "0001_initial"),
    ]

    operations = [
        migrations.AddField(
            model_name="session",
            name="note",
            field=models.CharField(blank=True, max_length=200),
        ),
        migrations.CreateModel(
            name="Membership",
            fields=[
                ("id", models.BigAutoField(auto_created=True, primary_key=True, serialize=False, verbose_name="ID")),
                ("kind", models.CharField(choices=[("monthly", "Monthly"), ("yearly", "Yearly"), ("guest", "Guest pass")], max_length=20)),
                ("valid_until", models.DateField()),
                ("member", models.OneToOneField(on_delete=django.db.models.deletion.CASCADE, related_name="membership", to=settings.AUTH_USER_MODEL)),
            ],
        ),
    ]
