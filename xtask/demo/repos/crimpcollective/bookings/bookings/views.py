from datetime import datetime, timedelta

from django.contrib import messages
from django.contrib.auth.decorators import login_required
from django.shortcuts import get_object_or_404, redirect, render
from django.utils import timezone
from django.utils.timezone import utc

from .emails import send_booked
from .models import Booking, Session

# Members may cancel until two hours before a session starts.
CANCEL_BEFORE = timedelta(hours=2)


def calendar(request):
    start = timezone.localdate()
    days = [start + timedelta(days=i) for i in range(14)]
    sessions = Session.objects.filter(starts_at__date__gte=start, starts_at__date__lt=start + timedelta(days=14))
    return render(request, "bookings/calendar.html", {"days": days, "sessions": sessions})


@login_required
def book(request, session_id):
    session = get_object_or_404(Session, pk=session_id)
    if request.method == "POST":
        if session.places_left <= 0:
            messages.error(request, "That session is full.")
        elif Booking.objects.filter(session=session, member=request.user, cancelled_at__isnull=True).exists():
            messages.info(request, "You've already booked it.")
        else:
            Booking.objects.create(session=session, member=request.user)
            send_booked(request.user, session)
            messages.success(request, f"Booked: {session}.")
        return redirect("calendar")
    return render(request, "bookings/session.html", {"session": session})


@login_required
def cancel(request, booking_id):
    booking = get_object_or_404(Booking, pk=booking_id, member=request.user)
    if booking.session.starts_at - datetime.now(tz=utc) < CANCEL_BEFORE:
        messages.error(request, "Bookings can be cancelled up to two hours before.")
    else:
        booking.cancelled_at = timezone.now()
        booking.save(update_fields=["cancelled_at"])
        messages.success(request, "Cancelled.")
    return redirect("calendar")
