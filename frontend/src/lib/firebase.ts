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

const app = initializeApp(firebaseConfig)
export const firebaseAuth = getAuth(app)
export const googleAuthProvider = new GoogleAuthProvider()
