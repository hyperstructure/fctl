from datetime import timedelta

from django.contrib.auth.models import User
from django.core import mail
from django.test import TestCase
from django.utils import timezone

from bookings.models import Booking, Session


class BookingTests(TestCase):
    def setUp(self):
        self.member = User.objects.create_user("maja", "maja@example.org", "climb-on")
        start = timezone.now() + timedelta(days=1)
        self.session = Session.objects.create(starts_at=start, ends_at=start + timedelta(hours=3), capacity=2)
        self.client.force_login(self.member)

    def test_booking_takes_a_place_and_confirms_it(self):
        self.client.post(f"/sessions/{self.session.id}/book")
        self.assertEqual(self.session.places_left, 1)
        self.assertEqual(mail.outbox[0].subject, f"Booked: {self.session}")

    def test_a_full_session_takes_no_more(self):
        for name in ["a", "b"]:
            Booking.objects.create(session=self.session, member=User.objects.create_user(name))
        self.client.post(f"/sessions/{self.session.id}/book")
        self.assertFalse(Booking.objects.filter(member=self.member).exists())

    def test_cancelling_too_late_is_refused(self):
        soon = timezone.now() + timedelta(hours=1)
        self.session.starts_at = soon
        self.session.save()
        booking = Booking.objects.create(session=self.session, member=self.member)
        self.client.post(f"/bookings/{booking.id}/cancel")
        booking.refresh_from_db()
        self.assertIsNone(booking.cancelled_at)
