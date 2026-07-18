import { describe, expect, it } from 'vitest'
import { render, screen } from '@testing-library/react'
import { CardGrid } from './CardGrid'
import { Card } from './Card'

describe('CardGrid + Card', () => {
  it('renders cards with a title and content inside the grid', () => {
    render(
      <CardGrid>
        <Card title="Open items">
          <p>3 items</p>
        </Card>
      </CardGrid>,
    )

    expect(screen.getByRole('heading', { name: 'Open items' })).toBeInTheDocument()
    expect(screen.getByText('3 items')).toBeInTheDocument()
  })

  it('does not crash with no children', () => {
    render(<CardGrid />)

    expect(document.body).toBeInTheDocument()
  })

  it('renders a Card with no title', () => {
    render(<Card>Just content</Card>)

    expect(screen.getByText('Just content')).toBeInTheDocument()
  })
})
