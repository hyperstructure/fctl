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
