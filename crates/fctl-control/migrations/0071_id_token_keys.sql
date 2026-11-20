-- Identity tokens (docs/identity-tokens.md): the instance's RSA keys that
-- sign the tokens agents mint, one at a time. Each is published by every
-- workspace's issuer two days before it signs and for two days after it
-- stops (fctl_domain::id_tokens::plan), and deleted then. `public_jwk` is
-- what the issuers list; `private_enc` the private key, a JWK sealed with
-- the master key for `id-token-key:<kid>`.
CREATE TABLE IF NOT EXISTS id_token_keys (
  kid TEXT PRIMARY KEY,
  public_jwk TEXT NOT NULL,
  private_enc TEXT NOT NULL,
  created_at INTEGER NOT NULL
);
