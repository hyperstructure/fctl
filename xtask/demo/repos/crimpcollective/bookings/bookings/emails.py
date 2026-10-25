from django.conf import settings
from django.core.mail import send_mail
from django.template.loader import render_to_string


def send_booked(member, session):
    """Confirms a booking, with the session's time and how to cancel."""
    send_mail(
        subject=f"Booked: {session}",
        message=render_to_string("bookings/emails/booked.txt", {"member": member, "session": session}),
        from_email=settings.DEFAULT_FROM_EMAIL,
        recipient_list=[member.email],
    )
