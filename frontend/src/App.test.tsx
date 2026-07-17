import { describe, expect, it } from 'vitest'
import { render, screen } from '@testing-library/react'
import App from './App'

// PROMPT-05 smoke test: proves the Vitest + React Testing Library harness runs
// end-to-end (ADR-013 layer 4). Not a real behavioral test — App has no logic yet.
describe('App', () => {
  it('renders the Cognitum Consultants heading', () => {
    render(<App />)

    expect(
      screen.getByRole('heading', { name: 'Cognitum Consultants' }),
    ).toBeInTheDocument()
  })
})
