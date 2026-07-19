import { describe, expect, it, vi } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import { Dialog } from './Dialog'

describe('Dialog', () => {
  it('renders nothing when open is false', () => {
    render(
      <Dialog open={false} onClose={vi.fn()} title="Confirm">
        <p>Are you sure?</p>
      </Dialog>,
    )

    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
  })

  it('renders with correct ARIA attributes when open is true', () => {
    render(
      <Dialog open={true} onClose={vi.fn()} title="Confirm">
        <p>Are you sure?</p>
      </Dialog>,
    )

    const dialog = screen.getByRole('dialog')
    expect(dialog).toHaveAttribute('aria-modal', 'true')
    expect(screen.getByText('Are you sure?')).toBeInTheDocument()
  })

  it('calls onClose when the backdrop is clicked', () => {
    const onClose = vi.fn()
    render(
      <Dialog open={true} onClose={onClose} title="Confirm">
        <p>Are you sure?</p>
      </Dialog>,
    )

    // The backdrop is the outer element rendered by the component; the
    // dialog itself stops propagation, so click the heading's grandparent.
    const dialog = screen.getByRole('dialog')
    const backdrop = dialog.parentElement
    expect(backdrop).not.toBeNull()
    fireEvent.click(backdrop as HTMLElement)

    expect(onClose).toHaveBeenCalledTimes(1)
  })

  it('does not call onClose when the dialog content is clicked', () => {
    const onClose = vi.fn()
    render(
      <Dialog open={true} onClose={onClose} title="Confirm">
        <p>Are you sure?</p>
      </Dialog>,
    )

    fireEvent.click(screen.getByRole('dialog'))

    expect(onClose).not.toHaveBeenCalled()
  })
})
