from datetime import date, timedelta

from django.contrib.auth.models import User
from django.test import TestCase

from bookings.models import Membership


class MembershipTests(TestCase):
    def test_a_guest_pass_names_its_kind(self):
        member = User.objects.create_user("jo")
        m = Membership.objects.create(member=member, kind="guest", valid_until=date.today())
        self.assertEqual(str(m), "jo (Guest pass)")

    def test_one_membership_per_member(self):
        member = User.objects.create_user("kim")
        Membership.objects.create(member=member, kind="monthly", valid_until=date.today() + timedelta(days=30))
        with self.assertRaises(Exception):
            Membership.objects.create(member=member, kind="yearly", valid_until=date.today())
