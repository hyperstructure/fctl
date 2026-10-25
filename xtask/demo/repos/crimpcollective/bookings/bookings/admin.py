from django.contrib import admin

from .models import Booking, Membership, Session


@admin.register(Session)
class SessionAdmin(admin.ModelAdmin):
    list_display = ["__str__", "capacity", "places_left", "note"]
    date_hierarchy = "starts_at"


@admin.register(Booking)
class BookingAdmin(admin.ModelAdmin):
    list_display = ["session", "member", "booked_at", "cancelled_at"]
    list_filter = ["session__starts_at"]


admin.site.register(Membership)
