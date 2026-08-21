/**
 * Tracks overlapping lint runs per target.
 *
 * A newer lint for the same directory cancels the previous child process and must not let a late
 * "superseded" finish overwrite the status bar with a stuck spinner.
 */

export type StatusUpdate = { type: 'noop' } | { type: 'set'; text: string; detail?: string }

export class LintRunCoordinator {
  private readonly generations = new Map<string, number>()
  private readonly controllers = new Map<string, AbortController>()
  private inflight = 0

  /**
   * Bump the generation for `target` and return a signal for the new child process.
   * Any previous run for the same target is aborted.
   */
  begin(target: string): { generation: number; signal: AbortSignal } {
    this.controllers.get(target)?.abort()

    const controller = new AbortController()
    this.controllers.set(target, controller)

    const generation = (this.generations.get(target) ?? 0) + 1
    this.generations.set(target, generation)

    return { generation, signal: controller.signal }
  }

  isCurrent(target: string, generation: number): boolean {
    return this.generations.get(target) === generation
  }

  markStarted(): void {
    this.inflight += 1
  }

  /**
   * Release one in-flight slot and decide how the status bar should update.
   *
   * When `superseded` is true, a newer run owns the UI — never publish a final spinner that says
   * the run was superseded (that is how the bar gets stuck).
   */
  markFinished(result: { superseded?: boolean; text?: string; detail?: string }): StatusUpdate {
    this.inflight = Math.max(0, this.inflight - 1)

    if (this.inflight > 0) {
      return {
        type: 'set',
        text: '$(sync~spin) slint',
        detail: `${this.inflight} lint run(s) still in progress`,
      }
    }

    if (result.superseded) {
      return { type: 'noop' }
    }

    return {
      type: 'set',
      text: result.text ?? 'slint',
      detail: result.detail,
    }
  }

  /** Soft release used when a static pass hands off to the model pass without leaving "idle". */
  releaseSlot(): void {
    this.inflight = Math.max(0, this.inflight - 1)
  }

  get inflightCount(): number {
    return this.inflight
  }
}
