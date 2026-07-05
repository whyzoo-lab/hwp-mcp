ALTER TABLE api_tokens ADD COLUMN IF NOT EXISTS expires_at timestamptz;
CREATE TABLE IF NOT EXISTS oauth_clients (
    client_id text PRIMARY KEY,
    redirect_uris text NOT NULL,
    client_name text,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE IF NOT EXISTS oauth_codes (
    code_hash text PRIMARY KEY,
    client_id text NOT NULL,
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    redirect_uri text NOT NULL,
    code_challenge text NOT NULL,
    resource text,
    expires_at timestamptz NOT NULL,
    used boolean NOT NULL DEFAULT false
);
CREATE TABLE IF NOT EXISTS oauth_refresh (
    token_hash text PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    client_id text NOT NULL,
    expires_at timestamptz NOT NULL,
    revoked_at timestamptz
);
