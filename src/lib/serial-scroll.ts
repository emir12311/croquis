/** Whether a scroll box is close enough to its bottom to keep following output. */
export interface ScrollMetrics {
  scrollTop: number
  clientHeight: number
  scrollHeight: number
}

/**
 * A few pixels of slack: fractional scroll positions and a scrollbar's rounding
 * would otherwise make "at the bottom" flicker false and stop following.
 */
export function isAtBottom(metrics: ScrollMetrics, slack = 8): boolean {
  return metrics.scrollHeight - metrics.scrollTop - metrics.clientHeight <= slack
}
