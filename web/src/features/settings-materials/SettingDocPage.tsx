/** A setting document editor: textarea plus a preview toggle, revision and history
 * (PROJECT §8.9). */
import { useState } from "react"
import { useParams } from "react-router"
import { useTranslation } from "react-i18next"
import ReactMarkdown from "react-markdown"
import remarkGfm from "remark-gfm"

import { useBook } from "@/api/hooks/books"
import { usePutSettingDoc, useSettingDocHistory, useSettingDocs } from "@/api/hooks/settings-materials"
import { useProposals } from "@/api/hooks/reviews"
import { ProposalCard } from "@/features/proposals/ProposalCard"
import { useErrorToast } from "@/components/common/ErrorToast"
import { EmptyState } from "@/components/common/EmptyState"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { Textarea } from "@/components/ui/textarea"
import { formatDateTime } from "@/lib/format"
import { isUnspaced } from "@/lib/language"

/** The document kinds this page renders. */
const KINDS = ["outline", "world", "protagonist"] as const
type DocKind = (typeof KINDS)[number]

export default function SettingDocPage() {
  const { bookId: rawBookId, doc: rawDoc } = useParams()
  const bookId = Number(rawBookId)
  const kind: DocKind = KINDS.includes(rawDoc as DocKind) ? (rawDoc as DocKind) : "outline"
  return <SettingDocEditor key={kind} bookId={bookId} kind={kind} />
}

export type SettingDocEditorProps = {
  bookId: number
  kind: DocKind
}

/** One document's editor, history and pending changes. */
export function SettingDocEditor({ bookId, kind }: SettingDocEditorProps) {
  const { t, i18n } = useTranslation()
  const showError = useErrorToast()
  const docs = useSettingDocs(bookId)
  const history = useSettingDocHistory(bookId, kind)
  const put = usePutSettingDoc(bookId, kind)
  const book = useBook(bookId)
  const proposals = useProposals(bookId, "pending")
  // Proposals for this document are shown inline above the editor (PROJECT §8.9).
  const pending = (proposals.data ?? []).filter(
    (proposal) => proposal.target_type === "setting_doc" && proposal.target_kind === kind,
  )
  const unspaced = isUnspaced(book.data?.writing_language ?? "en")
  const document = docs.data?.find((candidate) => candidate.kind === kind)
  // The draft is keyed by revision, so a background refetch never overwrites unsaved typing and
  // no effect has to copy the query into state.
  const [draft, setDraft] = useState<{ revision: number; content: string } | null>(null)
  const content =
    document === undefined
      ? ""
      : draft !== null && draft.revision === document.revision
        ? draft.content
        : document.content
  const setContent = (next: string) =>
    setDraft({ revision: document?.revision ?? 0, content: next })

  if (docs.isPending) {
    return <Skeleton className="m-6 h-64" />
  }
  if (docs.isError || document === undefined) {
    return (
      <div className="p-6">
        <EmptyState title={t("common:error_title")} description={t("common:error_body")} />
      </div>
    )
  }

  const dirty = content !== document.content

  return (
    <div className="mx-auto max-w-4xl space-y-4 p-6">
      <header className="flex items-end justify-between gap-4">
        <div>
          <h1 className="text-xl font-semibold">
            {t(`book:setting_doc.${kind}`, { defaultValue: kind })}
          </h1>
          <p className="mt-1 text-xs text-muted-foreground">
            {t("book:setting_doc_meta", {
              revision: document.revision,
              when: formatDateTime(document.updated_at, i18n.language),
            })}
          </p>
        </div>
        <div className="flex items-center gap-2">
          <Button
            size="sm"
            variant="outline"
            disabled={!dirty}
            onClick={() => setContent(document.content)}
          >
            {t("common:reset")}
          </Button>
          <Button
            size="sm"
            disabled={!dirty || put.isPending}
            onClick={() =>
              put.mutate(
                {
                  content,
                  base_revision: document.revision,
                  note: "",
                },
                { onError: showError },
              )
            }
          >
            {put.isPending ? t("common:saving") : t("common:save")}
          </Button>
        </div>
      </header>

      {pending.length === 0 ? null : (
        <section className="space-y-2">
          <h2 className="text-sm font-medium">{t("book:proposal.inline_title")}</h2>
          <div className="space-y-2">
            {pending.map((proposal) => (
              <ProposalCard
                key={proposal.id}
                proposal={proposal}
                bookId={bookId}
                unspaced={unspaced}
                compact
              />
            ))}
          </div>
        </section>
      )}

      <Tabs defaultValue="edit">
        <TabsList variant="line">
          <TabsTrigger value="edit">{t("common:edit")}</TabsTrigger>
          <TabsTrigger value="preview">{t("book:preview")}</TabsTrigger>
          <TabsTrigger value="history">{t("book:history")}</TabsTrigger>
        </TabsList>
        <TabsContent value="edit" className="pt-3">
          <Textarea
            value={content}
            rows={24}
            className="font-mono text-xs leading-relaxed"
            placeholder={t("book:setting_doc_placeholder")}
            onChange={(event) => setContent(event.target.value)}
          />
        </TabsContent>
        <TabsContent value="preview" className="pt-3">
          {content.trim() === "" ? (
            <EmptyState title={t("book:setting_doc_empty")} />
          ) : (
            <div className="prose-latin max-w-none space-y-3">
              <ReactMarkdown remarkPlugins={[remarkGfm]}>{content}</ReactMarkdown>
            </div>
          )}
        </TabsContent>
        <TabsContent value="history" className="pt-3">
          {(history.data ?? []).length === 0 ? (
            <EmptyState title={t("book:no_changes")} />
          ) : (
            <ul className="divide-y rounded-md border text-xs">
              {(history.data ?? []).map((change) => (
                <li key={change.id} className="flex items-center gap-2 px-3 py-2">
                  <span className="rounded bg-muted px-1.5 py-0.5">
                    {t(`book:change_op.${change.op}`, { defaultValue: change.op })}
                  </span>
                  <span className="text-muted-foreground">
                    {t(`book:change_actor.${change.actor}`, { defaultValue: change.actor })}
                  </span>
                  {change.note === "" ? null : <span>{change.note}</span>}
                  <span className="ml-auto text-muted-foreground">
                    {formatDateTime(change.created_at, i18n.language)}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </TabsContent>
      </Tabs>
    </div>
  )
}
