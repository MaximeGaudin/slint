import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import { RunFailureNotifier } from './run-failure.js'

describe('RunFailureNotifier (#30)', () => {
  it('notifies on the first "could not run" failure', () => {
    const notifier = new RunFailureNotifier()
    assert.equal(notifier.failureOccurred(), true)
  })

  it('does not re-notify on every failing save', () => {
    const notifier = new RunFailureNotifier()
    notifier.failureOccurred()
    assert.equal(notifier.failureOccurred(), false)
    assert.equal(notifier.failureOccurred(), false)
  })

  it('re-arms after a successful run so the next failure notifies again', () => {
    const notifier = new RunFailureNotifier()
    notifier.failureOccurred()
    assert.equal(notifier.failureOccurred(), false)

    notifier.runSucceeded()

    assert.equal(notifier.failureOccurred(), true)
    assert.equal(notifier.failureOccurred(), false)
  })

  it('stays armed across successes until a failure happens', () => {
    const notifier = new RunFailureNotifier()
    notifier.runSucceeded()
    notifier.runSucceeded()
    assert.equal(notifier.failureOccurred(), true)
  })
})
