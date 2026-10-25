from django.contrib import admin
from django.urls import include, path

urlpatterns = [
    path("admin/", admin.site.urls),
    path("routes/", include("routeboard.urls")),
    path("", include("bookings.urls")),
]
