/**
 * Pure helpers for surfacing a "slint could not run" failure.
 *
 * Kept free of the vscode module so unit tests can run under plain Node.
 */

/**
 * Decides when a "could not run" failure should raise a fresh, visible notification.
 *
 * A missing binary is the most common first-run failure, and lint runs on every save, so the
 * notification must not fire on every save. It fires once, then stays quiet until a run succeeds;
 * the next failure after a success raises it again.
 */
export class RunFailureNotifier {
  private armed = true

  /** Returns true when this failure should notify, and disarms until a run succeeds. */
  failureOccurred(): boolean {
    if (!this.armed) return false
    this.armed = false
    return true
  }

  /** A successful run re-arms the notification for the next failure. */
  runSucceeded(): void {
    this.armed = true
  }
}
