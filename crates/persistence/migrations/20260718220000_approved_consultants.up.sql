-- Email allowlist for real (Firebase-backed) login: an admin adds a row
-- here before a consultant can sign in with that email, per the "admin
-- approves specific non-cognitum.one addresses" requirement.
CREATE TABLE approved_consultants (
    email TEXT PRIMARY KEY,
    added_by TEXT NOT NULL,
    added_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
