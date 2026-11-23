-- How each sign-in was made (fctl_domain::sign_in, #618), so that the
-- domains an instance requires a provider of (InstanceSettings::sign_in)
-- take only the sign-ins that meet their rule, those made before it
-- included.
--
-- `auth_method`: `password`, `passkey`, or a provider's id (`google`);
-- `auth_domain`: for a provider's, the domain of the address it vouched
-- for. A CLI's sign-in takes them from the browser session that approved
-- it, and an access token from the sign-in that made it. NULL for the
-- sign-ins from before, which meet no rule.
ALTER TABLE tokens ADD COLUMN auth_method TEXT;
ALTER TABLE tokens ADD COLUMN auth_domain TEXT;

-- What the browser session that approved a CLI's sign-in kept, for the
-- CLI's token.
ALTER TABLE device_codes ADD COLUMN auth_method TEXT;
ALTER TABLE device_codes ADD COLUMN auth_domain TEXT;

-- A provider's sign-in that waits for its second factor: the domain of the
-- address the provider vouched for (`reason` says which provider).
ALTER TABLE sign_in_challenges ADD COLUMN auth_domain TEXT;

-- An admin token keeps it too, from the sign-in that made it: a domain's
-- rule holds the tokens its admins made as it holds their own.
ALTER TABLE admin_tokens ADD COLUMN auth_method TEXT;
ALTER TABLE admin_tokens ADD COLUMN auth_domain TEXT;

-- The domain of the address a provider's account vouched for at its last
-- sign-in (Google: one it hosts), NULL where it vouched for none: what
-- says whether that account can meet a domain's rule. `email` is what the
-- account shows, vouched for or not.
ALTER TABLE identities ADD COLUMN vouched_domain TEXT;
