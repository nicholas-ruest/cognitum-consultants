# syntax=docker/dockerfile:1
#
# Multi-stage build for cognitum-consultants (ADR-014: Deployment and
# Runtime Topology).
#
# Produces one deployable image: the compiled `bff-api` binary (ADR-003/
# ADR-004) serving both `/api/*` and the built frontend SPA (ADR-005/
# ADR-006) as static files at `/`. Postgres (ADR-010) is a provisioned
# dependency, not bundled here — see `docs/deployment.md`.
#
# Stages:
#   1. chef            - lukemathwalker/cargo-chef base (dependency-layer
#                         caching for the Rust build).
#   2. planner          - `cargo chef prepare`: computes a dependency
#                         "recipe" from the full workspace source tree.
#   3. builder          - `cargo chef cook` (builds/caches just the
#                         dependency graph — invalidated only when
#                         Cargo.toml/Cargo.lock change, not on every source
#                         edit), then `cargo build --release -p bff-api`.
#                         `SQLX_OFFLINE=true` because no live Postgres is
#                         reachable from inside a Docker build context;
#                         `sqlx`'s `query!`/`query_as!` macros type-check
#                         against the committed `.sqlx/` metadata instead
#                         (see `crates/persistence/README.md`).
#   4. frontend-builder - separate Node stage: `npm ci && npm run build` in
#                         `frontend/`, producing `frontend/dist/`.
#   5. runtime          - minimal `debian:bookworm-slim` (see trade-off note
#                         below), non-root, holding only the compiled
#                         binary + SPA assets + CA certs.

########################################
# 1-2. chef / planner
########################################
FROM lukemathwalker/cargo-chef:latest-rust-1-bookworm AS chef
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

########################################
# 3. builder
########################################
FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
# Dependency-only build, cached across builds whenever only application
# source (not Cargo.toml/Cargo.lock) changes.
RUN cargo chef cook --release --recipe-path recipe.json

# Real source, including the committed `.sqlx/` offline query-metadata
# cache (see module docs above) and `Cargo.lock`.
COPY . .
ENV SQLX_OFFLINE=true
RUN cargo build --release -p bff-api

########################################
# 4. frontend-builder
########################################
FROM node:24-slim AS frontend-builder
WORKDIR /app
# Real login (Google Sign-In via Firebase): Vite bakes `VITE_*` env vars
# into the built JS bundle at build time -- Cloud Run's runtime env vars
# never reach already-built static assets, so these must be build args
# instead. Values are the standard public Firebase *web* config (safe to
# embed client-side, not secrets in the security sense).
ARG VITE_FIREBASE_API_KEY
ARG VITE_FIREBASE_AUTH_DOMAIN
ARG VITE_FIREBASE_PROJECT_ID
ARG VITE_FIREBASE_APP_ID
ENV VITE_FIREBASE_API_KEY=$VITE_FIREBASE_API_KEY
ENV VITE_FIREBASE_AUTH_DOMAIN=$VITE_FIREBASE_AUTH_DOMAIN
ENV VITE_FIREBASE_PROJECT_ID=$VITE_FIREBASE_PROJECT_ID
ENV VITE_FIREBASE_APP_ID=$VITE_FIREBASE_APP_ID
# Copy every workspace member's manifest first so `npm ci` is cached
# separately from application source, same layer-caching intent as the Rust
# `cargo chef cook` step above. npm workspaces resolve from one root
# lockfile (PROMPT-42/ADR-017 moved this off a lone frontend/package-lock.json),
# so every member's package.json must be present for `npm ci` to succeed,
# not just frontend/'s.
COPY package.json package-lock.json ./
COPY frontend/package.json frontend/package.json
COPY packages/design-system/package.json packages/design-system/package.json
COPY packages/dashboard-components/package.json packages/dashboard-components/package.json
RUN npm ci
COPY frontend/ frontend/
COPY packages/ packages/
RUN npm run build --workspace=frontend

########################################
# 5. runtime
########################################
# Trade-off (ADR-014 leaves this choice open): `debian:bookworm-slim`
# rather than `gcr.io/distroless/cc`. Distroless would shrink the image and
# attack surface further (no shell, no package manager), but this stage
# still needs to create a non-root user and install `ca-certificates`
# (rustls-based outbound TLS to Nexus/Postgres needs the system trust
# store, ADR-007/ADR-010) — both trivial with `apt`/`useradd` here, and
# nontrivial to do *before* switching to a distroless base (no `useradd`
# there) without a more complex multi-stage user-provisioning dance.
# `slim` keeps the Dockerfile simple and debuggable (`docker exec` into a
# real shell) at the cost of a somewhat larger image / marginally larger
# surface than distroless; revisit if image size or CVE surface from the
# base OS packages becomes a real constraint.
FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home --uid 10001 --shell /usr/sbin/nologin appuser

WORKDIR /app

COPY --from=builder /app/target/release/bff-api /app/bff-api
COPY --from=frontend-builder /app/frontend/dist /app/frontend-dist

# ADR-014 12-factor env-var config (`crates/config`). `STATIC_DIR` is how
# `bff-api` finds the SPA build to serve (`crates/bff-api/src/main.rs`) —
# unset in any environment without a built frontend (e.g. the Rust test
# suite), set here because this image always has one.
ENV STATIC_DIR=/app/frontend-dist
ENV PORT=3000

USER appuser
EXPOSE 3000

ENTRYPOINT ["/app/bff-api"]
