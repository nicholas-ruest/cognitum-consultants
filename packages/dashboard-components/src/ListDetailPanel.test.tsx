import { describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen } from '@testing-library/react'
import { ListDetailPanel } from './ListDetailPanel'

interface Widget {
  id: string
  name: string
}

const WIDGETS: Widget[] = [
  { id: 'w1', name: 'Widget One' },
  { id: 'w2', name: 'Widget Two' },
]

describe('ListDetailPanel', () => {
  it('renders one row per item via renderRow', () => {
    render(
      <ListDetailPanel
        items={WIDGETS}
        getKey={(widget) => widget.id}
        renderRow={(widget) => <span>{widget.name}</span>}
      />,
    )

    expect(screen.getByText('Widget One')).toBeInTheDocument()
    expect(screen.getByText('Widget Two')).toBeInTheDocument()
  })

  it('updates the displayed detail when a row is clicked', () => {
    render(
      <ListDetailPanel
        items={WIDGETS}
        getKey={(widget) => widget.id}
        renderRow={(widget, { select }) => (
          <button type="button" onClick={select}>
            {widget.name}
          </button>
        )}
        renderDetail={(widget) => <p>Detail for {widget.name}</p>}
      />,
    )

    expect(screen.queryByText(/Detail for/)).not.toBeInTheDocument()

    fireEvent.click(screen.getByRole('button', { name: 'Widget Two' }))

    expect(screen.getByText('Detail for Widget Two')).toBeInTheDocument()
    expect(screen.queryByText('Detail for Widget One')).not.toBeInTheDocument()
  })

  it('calls onSelectedKeyChange with the clicked item key', () => {
    const onSelectedKeyChange = vi.fn()

    render(
      <ListDetailPanel
        items={WIDGETS}
        getKey={(widget) => widget.id}
        renderRow={(widget, { select }) => (
          <button type="button" onClick={select}>
            {widget.name}
          </button>
        )}
        onSelectedKeyChange={onSelectedKeyChange}
      />,
    )

    fireEvent.click(screen.getByRole('button', { name: 'Widget One' }))

    expect(onSelectedKeyChange).toHaveBeenCalledWith('w1')
  })

  it('renders nothing extra when no renderDetail prop is given, even after a click', () => {
    const { container } = render(
      <ListDetailPanel
        items={WIDGETS}
        getKey={(widget) => widget.id}
        renderRow={(widget, { select }) => (
          <button type="button" onClick={select}>
            {widget.name}
          </button>
        )}
      />,
    )

    fireEvent.click(screen.getByRole('button', { name: 'Widget One' }))

    // Only the <ul><li>...</li></ul> structure exists -- no Card/detail wrapper.
    expect(container.querySelectorAll('ul').length).toBe(1)
    expect(container.querySelectorAll('li').length).toBe(2)
  })

  it('supports controlled selection via selectedKey', () => {
    render(
      <ListDetailPanel
        items={WIDGETS}
        getKey={(widget) => widget.id}
        selectedKey="w2"
        renderRow={(widget, { isSelected }) => <span>{isSelected ? `${widget.name} (selected)` : widget.name}</span>}
        renderDetail={(widget) => <p>Detail for {widget.name}</p>}
      />,
    )

    expect(screen.getByText('Widget Two (selected)')).toBeInTheDocument()
    expect(screen.getByText('Detail for Widget Two')).toBeInTheDocument()
  })
})
