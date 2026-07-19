import { useEffect, useState } from 'react'
import type { ChangeEvent, FormEvent } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Alert, TextInput } from '@cognitum/design-system'
import { CapabilityForm } from '@cognitum/dashboard-components'
import type { CapabilityFormAlert } from '@cognitum/dashboard-components'
import { queryKeys } from '../../lib/queryKeys'
import { useSession } from '../../lib/SessionContext'

/**
 * PROMPT-36: Capacity profile-edit feature module, following
 * `ProposalWorkspace.tsx`'s combined `useQuery` (load) + `useMutation`
 * (submit) shape (`docs/SALES_FLOW_PATTERN.md` §4).
 *
 * Mirrors `crates/nexus-client/src/capacity.rs`'s `ConsultantProfileIntake`/
 * `ProfileUpdateResult` verbatim — `crates/bff-api/src/capacity.rs` relays
 * both unshaped, same convention as Sales' `AccountClaimResult` and Commit's
 * `ProposalSummary`.
 *
 * # Deliberately narrow: own profile only
 * `GET`/`PATCH /api/capacity/profile` (`crates/bff-api/src/capacity.rs`)
 * take no consultant-identifying input from this component at all — the BFF
 * derives the target consultant from the session cookie alone. This
 * component never renders, accepts, or transmits any other consultant's id
 * or data, matching the "restricted ACL" shape `anti-corruption-layers.md`
 * §4 and `domain-map.md` require.
 */
export interface ConsultantProfileIntake {
  skills: string[]
  certifications: string[]
  languages: string[]
  availability_window: string
  geographic_coverage: string[]
}

export interface ProfileUpdateResult {
  accepted: boolean
  reason: string | null
}

async function fetchOwnProfile(): Promise<ConsultantProfileIntake> {
  const response = await fetch('/api/capacity/profile', { credentials: 'include' })

  if (!response.ok) {
    throw new Error(`GET /api/capacity/profile failed: ${response.status}`)
  }

  return (await response.json()) as ConsultantProfileIntake
}

function updateOwnProfile(profile: ConsultantProfileIntake): Promise<ProfileUpdateResult> {
  return fetch('/api/capacity/profile', {
    method: 'PATCH',
    credentials: 'include',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(profile),
  }).then(async (response) => {
    if (!response.ok) {
      throw new Error(`PATCH /api/capacity/profile failed: ${response.status}`)
    }
    return (await response.json()) as ProfileUpdateResult
  })
}

/** Splits a comma-separated form field into a trimmed, non-empty array. */
function splitList(value: string): string[] {
  return value
    .split(',')
    .map((entry) => entry.trim())
    .filter((entry) => entry.length > 0)
}

/** Joins a fetched array field back into the form's comma-separated display shape. */
function joinList(values: string[]): string {
  return values.join(', ')
}

interface FormFields {
  skills: string
  certifications: string
  languages: string
  availabilityWindow: string
  geographicCoverage: string
}

function fieldsFromProfile(profile: ConsultantProfileIntake): FormFields {
  return {
    skills: joinList(profile.skills),
    certifications: joinList(profile.certifications),
    languages: joinList(profile.languages),
    availabilityWindow: profile.availability_window,
    geographicCoverage: joinList(profile.geographic_coverage),
  }
}

function profileFromFields(fields: FormFields): ConsultantProfileIntake {
  return {
    skills: splitList(fields.skills),
    certifications: splitList(fields.certifications),
    languages: splitList(fields.languages),
    availability_window: fields.availabilityWindow.trim(),
    geographic_coverage: splitList(fields.geographicCoverage),
  }
}

const EMPTY_FIELDS: FormFields = {
  skills: '',
  certifications: '',
  languages: '',
  availabilityWindow: '',
  geographicCoverage: '',
}

export function ProfileEditForm() {
  const session = useSession()
  const consultantId = session.status === 'authenticated' ? session.consultantId : undefined
  const queryClient = useQueryClient()

  const profileQuery = useQuery({
    queryKey: queryKeys.capacity.profile(consultantId ?? ''),
    queryFn: fetchOwnProfile,
    enabled: session.status === 'authenticated',
  })

  const [fields, setFields] = useState<FormFields>(EMPTY_FIELDS)

  // Seed the form once the fetched profile arrives — a later re-fetch (e.g.
  // after an accepted update invalidates the query below) also re-syncs the
  // form to whatever Capacity now reports as current, rather than leaving a
  // stale local edit displayed.
  useEffect(() => {
    if (profileQuery.data !== undefined) {
      setFields(fieldsFromProfile(profileQuery.data))
    }
  }, [profileQuery.data])

  const updateMutation = useMutation({
    mutationFn: updateOwnProfile,
    onSuccess: (result) => {
      // Only an accepted update actually changed Capacity's stored profile —
      // a rejection leaves it untouched, so only re-fetch (and thus
      // re-sync the form) on acceptance. Never re-derive `accepted` here;
      // this is purely "should we refresh", not a re-adjudication of the
      // verdict itself.
      if (result.accepted && consultantId !== undefined) {
        void queryClient.invalidateQueries({ queryKey: queryKeys.capacity.profile(consultantId) })
      }
    },
  })

  function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    // A fresh submission should never show a stale prior verdict while the
    // new one is in flight — same "reset before mutate" rule
    // `LeadConflictCheck.tsx`'s `handleSubmit` follows for ADR-015.
    updateMutation.reset()
    updateMutation.mutate(profileFromFields(fields))
  }

  function handleFieldChange(key: keyof FormFields) {
    return (event: ChangeEvent<HTMLInputElement>) => {
      setFields((current) => ({ ...current, [key]: event.target.value }))
    }
  }

  if (profileQuery.isPending) {
    return <p className="text-sm text-muted-foreground">Loading your profile…</p>
  }

  if (profileQuery.isError) {
    return <Alert variant="error">Failed to load your profile.</Alert>
  }

  const result = updateMutation.data

  const alerts: CapabilityFormAlert[] = [
    ...(result
      ? [
          {
            variant: result.accepted ? ('info' as const) : ('warning' as const),
            message: result.accepted
              ? 'Profile update accepted.'
              : `Profile update rejected${result.reason ? `: ${result.reason}` : '.'}`,
          },
        ]
      : []),
    ...(updateMutation.isError
      ? [{ variant: 'error' as const, message: 'Failed to submit your profile update. Please try again.' }]
      : []),
  ]

  return (
    <CapabilityForm
      alerts={alerts}
      onSubmit={handleSubmit}
      submitLabel="Save Profile"
      pendingLabel="Saving…"
      isPending={updateMutation.isPending}
    >
      <TextInput
        label="Skills (comma-separated)"
        value={fields.skills}
        onChange={handleFieldChange('skills')}
      />
      <TextInput
        label="Certifications (comma-separated)"
        value={fields.certifications}
        onChange={handleFieldChange('certifications')}
      />
      <TextInput
        label="Languages (comma-separated)"
        value={fields.languages}
        onChange={handleFieldChange('languages')}
      />
      <TextInput
        label="Availability Window"
        value={fields.availabilityWindow}
        onChange={handleFieldChange('availabilityWindow')}
      />
      <TextInput
        label="Geographic Coverage (comma-separated)"
        value={fields.geographicCoverage}
        onChange={handleFieldChange('geographicCoverage')}
      />
    </CapabilityForm>
  )
}
