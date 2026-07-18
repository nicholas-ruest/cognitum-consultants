import { describe, expect, it } from 'vitest'
import {
  actionQueueQueryKey,
  CAPABILITIES,
  capabilityKey,
  dashboardQueryKey,
  notificationsQueryKey,
  queryKeys,
} from './queryKeys'

// ADR-015 / PROMPT-16: pure tests for the query-key builder convention —
// no network needed. Assert shapes are correct (capability-namespaced,
// consultant-scoped) and stable (same inputs -> same key every call, so
// TanStack Query's key-equality caching works as expected).

describe('CAPABILITIES', () => {
  it('matches the nine frontend/src/features/<capability> directories', () => {
    expect(CAPABILITIES).toEqual([
      'sales',
      'commit',
      'edu',
      'capacity',
      'customer',
      'execution',
      'products',
      'landscape',
      'legal',
    ])
  })

  it('has a queryKeys namespace for every capability', () => {
    for (const capability of CAPABILITIES) {
      expect(queryKeys).toHaveProperty(capability)
    }
  })
})

describe('capabilityKey', () => {
  it('builds a [capability, resource, consultantId] tuple', () => {
    expect(capabilityKey('sales', 'conflicts', 'consultant-1')).toEqual([
      'sales',
      'conflicts',
      'consultant-1',
    ])
  })

  it('appends extra scoping segments after consultantId', () => {
    expect(capabilityKey('sales', 'account-claim', 'consultant-1', 'company-42')).toEqual([
      'sales',
      'account-claim',
      'consultant-1',
      'company-42',
    ])
  })

  it('is stable: identical inputs produce deep-equal (structurally, not referentially) keys', () => {
    const a = capabilityKey('commit', 'proposals', 'consultant-1')
    const b = capabilityKey('commit', 'proposals', 'consultant-1')

    expect(a).toEqual(b)
  })
})

describe('queryKeys.sales', () => {
  it('conflicts() matches ADR-015/PROMPT-16 example shape', () => {
    expect(queryKeys.sales.conflicts('consultant-1')).toEqual(['sales', 'conflicts', 'consultant-1'])
  })

  it('all is the capability root key', () => {
    expect(queryKeys.sales.all).toEqual(['sales'])
  })
})

describe('queryKeys.commit', () => {
  it('proposals() matches PROMPT-16 example shape', () => {
    expect(queryKeys.commit.proposals('consultant-1')).toEqual(['commit', 'proposals', 'consultant-1'])
  })
})

describe('generic capability resource()', () => {
  it('builds capability-namespaced keys for capabilities with no real routes yet', () => {
    expect(queryKeys.edu.resource('courses', 'consultant-1')).toEqual([
      'edu',
      'courses',
      'consultant-1',
    ])
    expect(queryKeys.legal.resource('contracts', 'consultant-1')).toEqual([
      'legal',
      'contracts',
      'consultant-1',
    ])
  })

  it('every non-sales/commit capability exposes all + resource', () => {
    const generic = CAPABILITIES.filter((c) => c !== 'sales' && c !== 'commit')

    for (const capability of generic) {
      const namespace = queryKeys[capability]
      expect(namespace.all).toEqual([capability])
      expect(namespace.resource('example', 'consultant-1')).toEqual([
        capability,
        'example',
        'consultant-1',
      ])
    }
  })
})

describe('dashboardQueryKey', () => {
  it('builds a [dashboard, consultantId] tuple, not namespaced under any capability', () => {
    expect(dashboardQueryKey('consultant-1')).toEqual(['dashboard', 'consultant-1'])
  })

  it('is stable: identical inputs produce deep-equal keys', () => {
    expect(dashboardQueryKey('consultant-1')).toEqual(dashboardQueryKey('consultant-1'))
  })

  it('scopes different consultants to different keys', () => {
    expect(dashboardQueryKey('consultant-1')).not.toEqual(dashboardQueryKey('consultant-2'))
  })
})

describe('notificationsQueryKey', () => {
  it('builds a [notifications, consultantId] tuple, not namespaced under any capability', () => {
    expect(notificationsQueryKey('consultant-1')).toEqual(['notifications', 'consultant-1'])
  })

  it('scopes different consultants to different keys', () => {
    expect(notificationsQueryKey('consultant-1')).not.toEqual(notificationsQueryKey('consultant-2'))
  })
})

describe('actionQueueQueryKey', () => {
  it('builds an [action-queue, consultantId] tuple, not namespaced under any capability', () => {
    expect(actionQueueQueryKey('consultant-1')).toEqual(['action-queue', 'consultant-1'])
  })

  it('scopes different consultants to different keys', () => {
    expect(actionQueueQueryKey('consultant-1')).not.toEqual(actionQueueQueryKey('consultant-2'))
  })
})
