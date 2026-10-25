from django.conf import settings
from django.db import models


class Session(models.Model):
    """A slot on the wall that members book: a weekday evening or a weekend morning."""

    starts_at = models.DateTimeField()
    ends_at = models.DateTimeField()
    capacity = models.PositiveSmallIntegerField(default=24)
    note = models.CharField(max_length=200, blank=True)

    class Meta:
        ordering = ["starts_at"]

    def __str__(self):
        return f"{self.starts_at:%a %d %b %H:%M}"

    @property
    def places_left(self):
        return self.capacity - self.bookings.filter(cancelled_at__isnull=True).count()


class Membership(models.Model):
    KINDS = [("monthly", "Monthly"), ("yearly", "Yearly"), ("guest", "Guest pass")]

    member = models.OneToOneField(settings.AUTH_USER_MODEL, on_delete=models.CASCADE, related_name="membership")
    kind = models.CharField(max_length=20, choices=KINDS)
    valid_until = models.DateField()

    def __str__(self):
        return f"{self.member} ({self.get_kind_display()})"


class Booking(models.Model):
    session = models.ForeignKey(Session, on_delete=models.CASCADE, related_name="bookings")
    member = models.ForeignKey(settings.AUTH_USER_MODEL, on_delete=models.CASCADE, related_name="bookings")
    booked_at = models.DateTimeField(auto_now_add=True)
    cancelled_at = models.DateTimeField(null=True, blank=True)

    class Meta:
        index_together = [("session", "member")]

    def __str__(self):
        return f"{self.member} on {self.session}"
