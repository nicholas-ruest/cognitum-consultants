#!/usr/bin/env bash
#
# manage-consultants.sh — add/remove/list approved consultant emails
# (the `approved_consultants` allowlist, `crates/auth/src/firebase.rs`).
#
# Access control is deliberately your own `gcloud` identity, not a shared
# password: this script fetches the app database credential from Secret
# Manager (`CONSULTANTS_APP_DB_PASSWORD`) via `gcloud secrets versions
# access`, which only succeeds if you're authenticated to `gcloud` *and*
# have `secretmanager.secretAccessor` on that secret. Whoever can run this
# script successfully is, by construction, someone with real GCP access to
# this project — the same bar as everything else in it. @cognitum.one
# addresses never need a row here at all (`login_with_id_token` always
# allows them) — this table is only for the non-cognitum.one consultant
# addresses that do need an explicit admin approval.
#
# Usage:
#   ./scripts/manage-consultants.sh add someone@example.com
#   ./scripts/manage-consultants.sh remove someone@example.com
#   ./scripts/manage-consultants.sh list
set -euo pipefail

DB_HOST="35.239.131.80"
DB_NAME="cognitum_consultants"
DB_USER="consultants_app"
SECRET_NAME="CONSULTANTS_APP_DB_PASSWORD"

if ! command -v gcloud >/dev/null 2>&1; then
    echo "gcloud is required. Install it and run 'gcloud auth login' first." >&2
    exit 1
fi

if ! command -v psql >/dev/null 2>&1; then
    echo "psql is required (postgresql-client)." >&2
    exit 1
fi

ACCOUNT="$(gcloud config get-value account 2>/dev/null || true)"
if [ -z "$ACCOUNT" ] || [ "$ACCOUNT" = "(unset)" ]; then
    echo "Not authenticated to gcloud. Run 'gcloud auth login' first." >&2
    exit 1
fi

DB_PASSWORD="$(gcloud secrets versions access latest --secret="$SECRET_NAME" 2>&1)" || {
    echo "Failed to read $SECRET_NAME from Secret Manager." >&2
    echo "You need secretmanager.secretAccessor on that secret — ask an admin to grant it," >&2
    echo "or (one-time bootstrap) create it: gcloud secrets create $SECRET_NAME --data-file=-" >&2
    exit 1
}

psql_cmd() {
    PGPASSWORD="$DB_PASSWORD" psql -h "$DB_HOST" -U "$DB_USER" -d "$DB_NAME" -v ON_ERROR_STOP=1 "$@"
}

usage() {
    echo "Usage: $0 add|remove|list [email]" >&2
    exit 1
}

case "${1:-}" in
    add)
        EMAIL="${2:-}"
        [ -n "$EMAIL" ] || usage
        # -v + :'var', fed via stdin (not -c, which skips psql's
        # variable-substitution preprocessing) so psql safely quotes the
        # value as a SQL literal -- an email containing a quote or SQL
        # metacharacter can't break out of the statement this way.
        psql_cmd -v email="$EMAIL" -v added_by="$ACCOUNT" <<'SQL'
INSERT INTO approved_consultants (email, added_by) VALUES (:'email', :'added_by') ON CONFLICT (email) DO NOTHING;
SQL
        echo "Approved: $EMAIL (added by $ACCOUNT)"
        ;;
    remove)
        EMAIL="${2:-}"
        [ -n "$EMAIL" ] || usage
        psql_cmd -v email="$EMAIL" <<'SQL'
DELETE FROM approved_consultants WHERE email = :'email';
SQL
        echo "Removed: $EMAIL"
        ;;
    list)
        psql_cmd -c "SELECT email, added_by, added_at FROM approved_consultants ORDER BY added_at;"
        ;;
    *)
        usage
        ;;
esac
