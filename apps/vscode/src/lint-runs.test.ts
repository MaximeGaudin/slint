import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import { LintRunCoordinator } from './lint-runs.js'

describe('LintRunCoordinator (#20)', () => {
  it('aborts the previous signal when a newer lint begins for the same target', () => {
    const coordinator = new LintRunCoordinator()
    const first = coordinator.begin('/skills/demo')
    assert.equal(first.signal.aborted, false)

    coordinator.begin('/skills/demo')

    assert.equal(
      first.signal.aborted,
      true,
      'starting a new lint must cancel the in-flight slint for that target',
    )
  })

  it('does not abort a run for a different target', () => {
    const coordinator = new LintRunCoordinator()
    const first = coordinator.begin('/skills/a')
    coordinator.begin('/skills/b')

    assert.equal(first.signal.aborted, false)
  })

  it('does not leave a superseded spinner after the newer run already finished', () => {
    const coordinator = new LintRunCoordinator()

    coordinator.begin('/skills/demo')
    coordinator.markStarted()

    coordinator.begin('/skills/demo')
    coordinator.markStarted()

    // Newer run finishes first while the old process is still winding down.
    const afterNewer = coordinator.markFinished({
      text: '$(check) slint',
      detail: 'No problems across 1 skill',
    })
    assert.deepEqual(afterNewer, {
      type: 'set',
      text: '$(sync~spin) slint',
      detail: '1 lint run(s) still in progress',
    })

    // Old run notices it was superseded. Must not clobber the newer result with a stuck spinner.
    const afterOlder = coordinator.markFinished({ superseded: true })
    assert.deepEqual(
      afterOlder,
      { type: 'noop' },
      'a superseded finish must not overwrite status once nothing else is in flight',
    )
  })

  it('still reports progress when a superseded run ends while another is running', () => {
    const coordinator = new LintRunCoordinator()

    coordinator.begin('/skills/demo')
    coordinator.markStarted()
    coordinator.begin('/skills/demo')
    coordinator.markStarted()

    const update = coordinator.markFinished({ superseded: true })
    assert.deepEqual(update, {
      type: 'set',
      text: '$(sync~spin) slint',
      detail: '1 lint run(s) still in progress',
    })
  })
})
