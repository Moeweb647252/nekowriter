import { useTranslation } from "react-i18next"

import { BarChart } from "./BarChart"
import type { WordsView } from "@/api/generated/WordsView"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { useFormatters } from "@/i18n/format"

/** Net word-count growth of chapter bodies in the range, and the words written so far. */
export function WordsCard({
  words,
  bucketLabel,
}: {
  words: WordsView
  bucketLabel: (index: number) => string
}) {
  const { t } = useTranslation()
  const format = useFormatters()
  return (
    <Card>
      <CardHeader>
        <CardTitle>{t(($) => $.dashboard.words.title)}</CardTitle>
        <CardDescription>{t(($) => $.dashboard.words.description)}</CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-6">
        <dl className="grid grid-cols-2 gap-4">
          <div>
            <dt className="text-sm text-muted-foreground">{t(($) => $.dashboard.words.net)}</dt>
            <dd className="font-heading text-3xl font-semibold">{format.signed(words.net)}</dd>
          </div>
          <div>
            <dt className="text-sm text-muted-foreground">{t(($) => $.dashboard.words.total)}</dt>
            <dd className="font-heading text-3xl font-semibold">{format.number(words.total)}</dd>
          </div>
        </dl>
        <BarChart
          label={t(($) => $.dashboard.words.chart)}
          values={words.buckets}
          bucketLabel={bucketLabel}
          valueLabel={format.signed}
        />
        {words.projects.length > 0 && (
          <table className="w-full text-sm">
            <thead className="text-left text-muted-foreground">
              <tr>
                <th className="py-1 font-normal">{t(($) => $.dashboard.work)}</th>
                <th className="py-1 text-right font-normal">{t(($) => $.dashboard.words.net)}</th>
              </tr>
            </thead>
            <tbody>
              {words.projects.map((project) => (
                <tr key={project.projectId} className="border-t">
                  <td className="py-1">{project.name}</td>
                  <td className="py-1 text-right tabular-nums">{format.signed(project.net)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </CardContent>
    </Card>
  )
}
