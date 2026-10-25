from django.urls import path

from . import views

urlpatterns = [
    path("", views.calendar, name="calendar"),
    path("sessions/<int:session_id>/book", views.book, name="book"),
    path("bookings/<int:booking_id>/cancel", views.cancel, name="cancel"),
]
