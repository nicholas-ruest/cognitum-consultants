-- Persisted sessions for the real (Firebase-backed) login provider, so a
-- consultant stays signed in across a BFF instance restart/rescale
-- (Cloud Run scales to zero between requests) -- unlike the in-memory
-- dev-stub sessions, which never need to survive a process restart.
CREATE TABLE sessions (
    session_id UUID PRIMARY KEY,
    consultant_id TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX sessions_expires_at_idx ON sessions (expires_at);
