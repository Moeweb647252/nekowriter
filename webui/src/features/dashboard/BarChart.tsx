/**
 * Bars for a series of values that may be negative, drawn from a shared baseline. Each
 * bar names its bucket and value, so the chart reads without seeing it.
 */
export function BarChart({
  label,
  values,
  bucketLabel,
  valueLabel,
}: {
  label: string
  values: readonly number[]
  bucketLabel: (index: number) => string
  valueLabel: (value: number) => string
}) {
  const largest = Math.max(1, ...values.map((value) => Math.abs(value)))
  const hasNegative = values.some((value) => value < 0)
  return (
    <ol aria-label={label} className="flex h-32 items-stretch gap-px">
      {values.map((value, index) => {
        const share = `${String((Math.abs(value) / largest) * 100)}%`
        return (
          // A bucket's place on the axis is its identity.
          <li
            key={index}
            className="flex min-w-0 flex-1 flex-col"
            title={`${bucketLabel(index)}: ${valueLabel(value)}`}
          >
            <span className="sr-only">{`${bucketLabel(index)}: ${valueLabel(value)}`}</span>
            <div className="flex flex-1 flex-col justify-end">
              {value > 0 && <div className="rounded-t-sm bg-primary" style={{ height: share }} />}
            </div>
            {hasNegative && (
              <div className="flex flex-1 flex-col border-t border-border">
                {value < 0 && (
                  <div className="rounded-b-sm bg-destructive/70" style={{ height: share }} />
                )}
              </div>
            )}
          </li>
        )
      })}
    </ol>
  )
}
