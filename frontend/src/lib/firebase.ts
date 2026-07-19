import { initializeApp } from 'firebase/app'
import { GoogleAuthProvider, getAuth } from 'firebase/auth'

/**
 * Real login (Google Sign-In via Firebase), replacing the dev-stub outside
 * a dev environment. This is the standard public Firebase *web* config —
 * safe to embed client-side (it identifies the project, it isn't a
 * secret) — sourced from Vite env vars set at build/deploy time.
 *
 * Access is still gated server-side: `POST /api/login/firebase`
 * (`crates/auth/src/firebase.rs`) only issues a session if the signed-in
 * account's email is in the `approved_consultants` allowlist. Signing in
 * with Google here proves identity, not authorization.
 */
const firebaseConfig = {
  apiKey: import.meta.env.VITE_FIREBASE_API_KEY,
  authDomain: import.meta.env.VITE_FIREBASE_AUTH_DOMAIN,
  projectId: import.meta.env.VITE_FIREBASE_PROJECT_ID,
  appId: import.meta.env.VITE_FIREBASE_APP_ID,
}

// `initializeApp`/`getAuth` throw synchronously (`auth/invalid-api-key`) the
// moment `apiKey` is empty -- true for every build without Firebase baked in
// (plain `npm run dev`, the e2e suite, any `bff-api` dev-stub-only
// environment). `LoginPage.tsx` imports `firebaseAuth` unconditionally, so
// without this guard that throw happens at import time and crashes the
// whole app before `LoginPage`'s own `firebaseConfigured` fallback logic
// ever gets a chance to run. `loginWithGoogle` there only dereferences
// `firebaseAuth` when `firebaseConfigured` is true, so it's non-null on
// every reachable call.
export const firebaseAuth = firebaseConfig.apiKey ? getAuth(initializeApp(firebaseConfig)) : null
export const googleAuthProvider = new GoogleAuthProvider()
