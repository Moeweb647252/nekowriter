import { useTranslation } from "react-i18next"

import { BarChart } from "./BarChart"
import type { UsageTotals } from "@/api/generated/UsageTotals"
import type { UsageView } from "@/api/generated/UsageView"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { useFormatters } from "@/i18n/format"

/** The tokens of the requests whose usage was reported. */
function knownTokens(totals: UsageTotals): number {
  return totals.inputTokens + totals.outputTokens
}

function unknownRequests(totals: UsageTotals): number {
  return totals.pending + totals.notReported + totals.noAnswer
}

/** Token usage by role, time and work; requests with unknown usage are counted apart. */
export function UsageCard({
  usage,
  bucketLabel,
}: {
  usage: UsageView
  bucketLabel: (index: number) => string
}) {
  const { t } = useTranslation()
  const format = useFormatters()
  const roles = [
    { key: "writing", totals: usage.writing },
    { key: "review", totals: usage.review },
    { key: "connectionTests", totals: usage.connectionTests },
  ] as const
  const unknown = roles.reduce(
    (sum, role) => ({
      pending: sum.pending + role.totals.pending,
      notReported: sum.notReported + role.totals.notReported,
      noAnswer: sum.noAnswer + role.totals.noAnswer,
      invalid: sum.invalid + role.totals.invalid,
    }),
    { pending: 0, notReported: 0, noAnswer: 0, invalid: 0 },
  )
  const columns = [
    t(($) => $.dashboard.usage.requests),
    t(($) => $.dashboard.usage.input),
    t(($) => $.dashboard.usage.cachedInput),
    t(($) => $.dashboard.usage.output),
    t(($) => $.dashboard.usage.reasoning),
    t(($) => $.dashboard.usage.known),
    t(($) => $.dashboard.usage.unknown),
  ]

  return (
    <Card>
      <CardHeader>
        <CardTitle>{t(($) => $.dashboard.usage.title)}</CardTitle>
        <CardDescription>{t(($) => $.dashboard.usage.description)}</CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-6">
        <div className="overflow-x-auto">
          <table className="w-full text-sm">
            <thead className="text-left text-muted-foreground">
              <tr>
                <th className="py-1 font-normal">{t(($) => $.dashboard.usage.role)}</th>
                {columns.map((column) => (
                  <th key={column} className="py-1 pl-4 text-right font-normal">
                    {column}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {roles.map(({ key, totals }) => (
                <tr key={key} className="border-t">
                  <th scope="row" className="py-1 text-left font-normal">
                    {t(($) => $.dashboard.usage.roles[key])}
                  </th>
                  {[
                    totals.requests,
                    totals.inputTokens,
                    totals.cachedInputTokens,
                    totals.outputTokens,
                    totals.reasoningTokens,
                    knownTokens(totals),
                    unknownRequests(totals) + totals.invalid,
                  ].map((value, index) => (
                    // The columns are fixed; a cell's place is its identity.
                    <td key={index} className="py-1 pl-4 text-right tabular-nums">
                      {format.number(value)}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        {unknown.pending + unknown.notReported + unknown.noAnswer + unknown.invalid > 0 && (
          <p className="text-sm text-muted-foreground">
            {t(($) => $.dashboard.usage.unknownNote, {
              pending: format.number(unknown.pending),
              notReported: format.number(unknown.notReported),
              noAnswer: format.number(unknown.noAnswer),
              invalid: format.number(unknown.invalid),
            })}
          </p>
        )}
        <BarChart
          label={t(($) => $.dashboard.usage.chart)}
          values={usage.buckets.map(knownTokens)}
          bucketLabel={bucketLabel}
          valueLabel={format.number}
        />
        {usage.projects.length > 0 && (
          <table className="w-full text-sm">
            <thead className="text-left text-muted-foreground">
              <tr>
                <th className="py-1 font-normal">{t(($) => $.dashboard.work)}</th>
                <th className="py-1 pl-4 text-right font-normal">
                  {t(($) => $.dashboard.usage.roles.writing)}
                </th>
                <th className="py-1 pl-4 text-right font-normal">
                  {t(($) => $.dashboard.usage.roles.review)}
                </th>
                <th className="py-1 pl-4 text-right font-normal">
                  {t(($) => $.dashboard.usage.unknown)}
                </th>
              </tr>
            </thead>
            <tbody>
              {usage.projects.map((project) => (
                <tr key={project.projectId} className="border-t">
                  <td className="py-1">{project.name}</td>
                  <td className="py-1 pl-4 text-right tabular-nums">
                    {format.number(knownTokens(project.writing))}
                  </td>
                  <td className="py-1 pl-4 text-right tabular-nums">
                    {format.number(knownTokens(project.review))}
                  </td>
                  <td className="py-1 pl-4 text-right tabular-nums">
                    {format.number(
                      unknownRequests(project.writing) +
                        project.writing.invalid +
                        unknownRequests(project.review) +
                        project.review.invalid,
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </CardContent>
    </Card>
  )
}
