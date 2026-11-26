-- A workspace's own sign-in requirement (#618, ADR 0103): its members who
-- are in it as an address at one of the rule's domains open it only with a
-- sign-in made with its provider as such an address. JSON of a
-- fctl_proto::api::SignInRule ({"domains": [...], "provider": "google",
-- "passkeys": false}); NULL, the default: none.
ALTER TABLE workspaces ADD COLUMN sign_in TEXT;
