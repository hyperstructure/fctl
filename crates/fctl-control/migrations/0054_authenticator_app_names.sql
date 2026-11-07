-- The name a person gives their authenticator app in Settings ("1Password",
-- "Work phone"), since nothing about a TOTP secret says which app keeps
-- it. NULL until they name it, and again once they replace the app: the
-- web app and the CLI call it "Authenticator app" meanwhile.
ALTER TABLE authenticator_apps ADD COLUMN name TEXT;
