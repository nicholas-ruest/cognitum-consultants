import { execFile, spawn } from 'node:child_process'
import type { ChildProcess } from 'node:child_process'
import { promisify } from 'node:util'
import { readdirSync, readFileSync } from 'node:fs'
import { join, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'
import { BFF_PORT, POSTGRES_DATABASE_URL, POSTGRES_PORT } from './constants'

/**
 * Orchestrates the real backend half of the PROMPT-27 e2e stack: a
 * throwaway Postgres container (migrated), plus the real `bff-api` Rust
 * binary pointed at it and at a (separately started, see
 * `mock-nexus-server.ts`) mock Nexus. This is the same "real BFF, real
 * Postgres, Nexus mocked at the HTTP boundary" pattern manual verification
 * used in PROMPT-18/19/23, formalized as a reusable module so
 * `global-setup.ts` stays a thin composition of this module +
 * `mock-nexus-server.ts`, and so Phase 4 (PROMPT-34+) e2e specs can reuse
 * it unchanged.
 *
 * Postgres is started via a plain `docker run` (not `testcontainers`,
 * which is a Rust/JVM-ecosystem library with no first-class Node API) —
 * this mirrors the "GitHub-hosted `ubuntu-latest` runners ship Docker
 * Engine preinstalled" assumption `.github/workflows/ci.yml`'s `rust` job
 * already relies on for its own `testcontainers`-based tests.
 */

const execFileAsync = promisify(execFile)

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), '..', '..', '..')
const MIGRATIONS_DIR = join(REPO_ROOT, 'crates', 'persistence', 'migrations')
const BFF_BINARY_PATH = join(REPO_ROOT, 'target', 'debug', 'bff-api')
const POSTGRES_CONTAINER_NAME = 'cognitum-e2e-postgres'
const POSTGRES_IMAGE = 'postgres:17-alpine'

/** `cargo`/`sqlx` may only be on PATH via `~/.cargo/bin` (not sourced into
 * a non-login shell's PATH by default) — extend PATH defensively rather
 * than assume the invoking shell already has it, same rationale as
 * `scripts/dev.sh`'s `~/.cargo/env` sourcing. */
function envWithCargoOnPath(): NodeJS.ProcessEnv {
  const cargoBin = join(process.env.HOME ?? '', '.cargo', 'bin')
  return { ...process.env, PATH: `${cargoBin}:${process.env.PATH ?? ''}` }
}

async function sleep(ms: number): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, ms))
}

async function waitFor(description: string, timeoutMs: number, check: () => Promise<boolean>): Promise<void> {
  const deadline = Date.now() + timeoutMs
  let lastError: unknown
  while (Date.now() < deadline) {
    try {
      if (await check()) return
    } catch (err) {
      lastError = err
    }
    await sleep(250)
  }
  throw new Error(`timed out waiting for ${description}${lastError ? `: ${String(lastError)}` : ''}`)
}

/** Removes any stale container left over from a previous crashed run, so
 * `docker run --name ...` below never fails with "name already in use". */
async function removeStaleContainer(): Promise<void> {
  await execFileAsync('docker', ['rm', '-f', POSTGRES_CONTAINER_NAME]).catch(() => undefined)
}

async function startPostgresContainer(): Promise<void> {
  await removeStaleContainer()
  await execFileAsync('docker', [
    'run',
    '-d',
    '--rm',
    '--name',
    POSTGRES_CONTAINER_NAME,
    '-e',
    'POSTGRES_PASSWORD=postgres',
    '-p',
    `${POSTGRES_PORT}:5432`,
    POSTGRES_IMAGE,
  ])

  // Neither `pg_isready` nor a real `psql` query is a sufficient readiness
  // signal here: the official postgres image's entrypoint starts a fully
  // functional *transient* server to run `initdb`/init scripts against
  // (over the same Unix socket this container exposes), which answers
  // `pg_isready` AND genuine SQL queries successfully, then shuts that
  // instance down and starts the real, final server. A caller that
  // proceeds right after either check passes can race that shutdown and
  // find no socket at all for a brief window (observed in CI as
  // "connection to server on socket ... failed: No such file or
  // directory", on both the readiness check itself and, once, on the very
  // first migration after a readiness check had already reported success).
  //
  // The one unambiguous signal this entrypoint gives for "the real server
  // is up" is its own log line: "database system is ready to accept
  // connections" is printed once for the transient instance and a second
  // time for the final one (see the postgres image's docker-entrypoint.sh).
  // Waiting for that line to appear twice in `docker logs`, THEN
  // confirming with a real query, closes the race the two previous
  // (insufficient) attempts at this fix did not.
  await waitFor('Postgres init to reach the final server instance', 60_000, async () => {
    const { stdout, stderr } = await execFileAsync('docker', ['logs', POSTGRES_CONTAINER_NAME])
    const occurrences = ((stdout + stderr).match(/database system is ready to accept connections/g) ?? []).length
    return occurrences >= 2
  })

  await waitFor('Postgres to accept connections', 60_000, async () => {
    await execFileAsync('docker', [
      'exec',
      POSTGRES_CONTAINER_NAME,
      'psql',
      '-U',
      'postgres',
      '-d',
      'postgres',
      '-c',
      'SELECT 1',
    ])
    return true
  })
}

/** Applies every `*.up.sql` migration directly via `psql` inside the
 * container, in filename (timestamp-prefixed, so chronological) order.
 * Raw SQL rather than `sqlx migrate run` — `bff-api` doesn't read sqlx's
 * migration-history table at runtime (see `crates/persistence/README.md`:
 * "not yet wired" for auto-migration), so this only needs the schema to
 * exist, not a tracked migration ledger; it also sidesteps depending on
 * `sqlx-cli` being installed in every environment this runs in (dev
 * sandbox, CI). */
async function applyMigrations(): Promise<void> {
  const files = readdirSync(MIGRATIONS_DIR)
    .filter((name) => name.endsWith('.up.sql'))
    .sort()

  for (const file of files) {
    const sql = readFileSync(join(MIGRATIONS_DIR, file), 'utf8')
    await new Promise<void>((resolve, reject) => {
      const psql = spawn('docker', [
        'exec',
        '-i',
        POSTGRES_CONTAINER_NAME,
        'psql',
        '-v',
        'ON_ERROR_STOP=1',
        '-U',
        'postgres',
        '-d',
        'postgres',
      ])
      let stderr = ''
      psql.stderr.on('data', (chunk: Buffer) => {
        stderr += chunk.toString()
      })
      psql.on('error', reject)
      psql.on('close', (code) => {
        if (code === 0) resolve()
        else reject(new Error(`applying migration ${file} failed (exit ${code}): ${stderr}`))
      })
      psql.stdin.end(sql)
    })
  }
}

async function stopPostgresContainer(): Promise<void> {
  await execFileAsync('docker', ['stop', POSTGRES_CONTAINER_NAME]).catch(() => undefined)
}

/** `cargo build -p bff-api` ahead of `spawn`ing the resulting binary
 * directly — faster and quieter than `cargo run` (which re-invokes the
 * build step and interleaves its own output with the running server's). */
async function buildBffApi(): Promise<void> {
  await execFileAsync('cargo', ['build', '-p', 'bff-api'], {
    cwd: REPO_ROOT,
    env: envWithCargoOnPath(),
    maxBuffer: 64 * 1024 * 1024,
  })
}

function startBffApi(nexusEndpointUrl: string): ChildProcess {
  return spawn(BFF_BINARY_PATH, [], {
    cwd: REPO_ROOT,
    env: {
      ...envWithCargoOnPath(),
      DATABASE_URL: POSTGRES_DATABASE_URL,
      NEXUS_ENDPOINT_URL: nexusEndpointUrl,
      PORT: String(BFF_PORT),
      APP_ENV: 'dev',
      RUST_LOG: process.env.RUST_LOG ?? 'info',
    },
    stdio: ['ignore', 'pipe', 'pipe'],
  })
}

async function waitForBffHealthz(): Promise<void> {
  await waitFor('bff-api /healthz to respond', 30_000, async () => {
    const response = await fetch(`http://127.0.0.1:${BFF_PORT}/healthz`)
    return response.ok
  })
}

export interface TestStack {
  stop: () => Promise<void>
}

/**
 * Brings up Postgres (migrated) and `bff-api` (pointed at `nexusEndpointUrl`,
 * expected to already be a running mock Nexus server — see
 * `mock-nexus-server.ts`). Resolves once `bff-api`'s `/healthz` responds
 * successfully, i.e. the whole backend is ready for Playwright to drive
 * the frontend against.
 */
export async function startTestStack(nexusEndpointUrl: string): Promise<TestStack> {
  await startPostgresContainer()
  await applyMigrations()
  await buildBffApi()

  const bffProcess = startBffApi(nexusEndpointUrl)
  let bffOutput = ''
  bffProcess.stdout?.on('data', (chunk: Buffer) => {
    bffOutput += chunk.toString()
  })
  bffProcess.stderr?.on('data', (chunk: Buffer) => {
    bffOutput += chunk.toString()
  })

  let bffExitedEarly: number | null | undefined
  bffProcess.once('exit', (code) => {
    bffExitedEarly = code
  })

  try {
    await waitForBffHealthz()
  } catch (err) {
    throw new Error(
      `bff-api failed to become healthy (exited early: ${String(bffExitedEarly)}): ${String(err)}\n--- bff-api output ---\n${bffOutput}`,
    )
  }

  return {
    stop: async () => {
      if (bffProcess.exitCode === null && bffProcess.signalCode === null) {
        bffProcess.kill('SIGTERM')
        await new Promise<void>((resolve) => {
          bffProcess.once('exit', () => resolve())
          // Don't hang teardown forever if the process ignores SIGTERM.
          setTimeout(resolve, 5_000)
        })
      }
      await stopPostgresContainer()
    },
  }
}
