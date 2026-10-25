# Crimp Collective bookings

Session bookings, memberships and the route board for the club's wall.
Django, kept alive by volunteers: if something breaks, write to Elsa.

```sh
python -m venv .venv && . .venv/bin/activate
pip install -r requirements.txt
python manage.py migrate
python manage.py runserver     # http://localhost:8000, the admin at /admin
python manage.py test
```

Members book sessions on the calendar. Admins add sessions, memberships and
routes in the admin. The screen by the wall shows `/routes/board`.
