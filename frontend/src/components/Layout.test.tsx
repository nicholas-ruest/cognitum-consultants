import { describe, expect, it } from 'vitest'
import { render, screen } from '@testing-library/react'
import { Layout } from './Layout'

describe('Layout', () => {
  it('renders sidebar and main content without throwing', () => {
    render(
      <Layout sidebar={<div>Sidebar content</div>}>
        <p>Main content</p>
      </Layout>,
    )

    expect(screen.getByText('Sidebar content')).toBeInTheDocument()
    expect(screen.getByText('Main content')).toBeInTheDocument()
  })

  it('does not crash with no props', () => {
    render(<Layout />)

    expect(document.body).toBeInTheDocument()
  })
})
