import { type SubmitEvent, useState } from "react"
import { useTranslation } from "react-i18next"

import type { EditorTarget } from "./KnowledgePage"
import { errorForField, errorForForm } from "@/api/fieldErrors"
import type { EntryView } from "@/api/generated/EntryView"
import type { KnowledgeEntryId } from "@/api/generated/KnowledgeEntryId"
import type { KnowledgeSpaceId } from "@/api/generated/KnowledgeSpaceId"
import type { SpaceView } from "@/api/generated/SpaceView"
import {
  useCreateEntry,
  useDeleteEntry,
  useEntry,
  useReindexEntry,
  useUpdateEntry,
} from "@/api/queries/knowledge"
import { ErrorAlert } from "@/components/app/ErrorAlert"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog"
import { Button } from "@/components/ui/button"
import { Field, FieldDescription, FieldError, FieldGroup, FieldLabel } from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Skeleton } from "@/components/ui/skeleton"
import { Textarea } from "@/components/ui/textarea"
import { useEmbeddingFailureText, useErrorText } from "@/i18n/errors"
import { cn } from "@/lib/utils"

const FIELDS = ["entry_title", "entry_content", "keyword"] as const

/** Keywords as the user types them: separated by ASCII, full-width or ideographic commas. */
function splitKeywords(text: string): string[] {
  return text.split(/[,\uff0c\u3001]/)
}

/** The editor for the target: a new entry, an existing one once it loads, or a hint. */
export function EntryEditor({
  spaces,
  defaultSpace,
  target,
  onSaved,
  onDeleted,
}: {
  spaces: readonly SpaceView[]
  defaultSpace: KnowledgeSpaceId | null
  target: EditorTarget
  onSaved: (id: KnowledgeEntryId) => void
  onDeleted: () => void
}) {
  const { t } = useTranslation()
  const entry = useEntry(target.kind === "entry" ? target.id : null)
  const firstSpace = defaultSpace ?? spaces[0]?.id
  const hint = <p className="text-sm text-muted-foreground">{t(($) => $.knowledge.select)}</p>
  if (target.kind === "none") return hint
  if (target.kind === "new") {
    return firstSpace === undefined ? (
      hint
    ) : (
      <EntryForm
        spaces={spaces}
        entry={null}
        space={firstSpace}
        onSaved={onSaved}
        onDeleted={onDeleted}
      />
    )
  }
  if (entry.isError) return <ErrorAlert error={entry.error} onRetry={() => void entry.refetch()} />
  if (entry.data === undefined) return <Skeleton className="h-96" />
  return (
    <EntryForm
      spaces={spaces}
      entry={entry.data}
      space={entry.data.spaceId}
      onSaved={onSaved}
      onDeleted={onDeleted}
    />
  )
}

function EntryForm({
  spaces,
  entry,
  space: initialSpace,
  onSaved,
  onDeleted,
}: {
  spaces: readonly SpaceView[]
  entry: EntryView | null
  space: KnowledgeSpaceId
  onSaved: (id: KnowledgeEntryId) => void
  onDeleted: () => void
}) {
  const { t } = useTranslation()
  const errorText = useErrorText()
  const create = useCreateEntry()
  const update = useUpdateEntry()
  const remove = useDeleteEntry()
  const [space, setSpace] = useState(initialSpace)
  const [title, setTitle] = useState(entry?.title ?? "")
  const [keywords, setKeywords] = useState(entry?.keywords.join(", ") ?? "")
  const [content, setContent] = useState(entry?.content ?? "")
  const save = entry === null ? create : update
  const error = save.error ?? remove.error
  const items = spaces.map((item) => ({ value: item.id, label: item.name }))

  const submit = (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    const fields = { spaceId: space, title, keywords: splitKeywords(keywords), content }
    const done = { onSuccess: (saved: EntryView) => onSaved(saved.id) }
    if (entry === null) create.mutate(fields, done)
    else update.mutate({ entry, fields }, done)
  }

  const titleError = errorForField(error, "entry_title")
  const keywordsError = errorForField(error, "keyword")
  const contentError = errorForField(error, "entry_content")
  const formError = errorForForm(error, FIELDS)
  return (
    <form onSubmit={submit} className="flex min-w-0 flex-col gap-4">
      <FieldGroup>
        <Field>
          <FieldLabel htmlFor="entry-space">{t(($) => $.knowledge.entry.space)}</FieldLabel>
          <Select
            items={items}
            value={space}
            onValueChange={(value: KnowledgeSpaceId | null) => {
              if (value !== null) setSpace(value)
            }}
          >
            <SelectTrigger id="entry-space" className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {items.map((item) => (
                <SelectItem key={item.value} value={item.value}>
                  {item.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </Field>
        <Field data-invalid={titleError !== undefined}>
          <FieldLabel htmlFor="entry-title">{t(($) => $.knowledge.entry.title)}</FieldLabel>
          <Input
            id="entry-title"
            value={title}
            onChange={(event) => setTitle(event.target.value)}
            aria-invalid={titleError !== undefined}
          />
          {titleError !== undefined && <FieldError>{errorText(titleError)}</FieldError>}
        </Field>
        <Field data-invalid={keywordsError !== undefined}>
          <FieldLabel htmlFor="entry-keywords">{t(($) => $.knowledge.entry.keywords)}</FieldLabel>
          <Input
            id="entry-keywords"
            value={keywords}
            onChange={(event) => setKeywords(event.target.value)}
            aria-invalid={keywordsError !== undefined}
          />
          <FieldDescription>{t(($) => $.knowledge.entry.keywordsHint)}</FieldDescription>
          {keywordsError !== undefined && <FieldError>{errorText(keywordsError)}</FieldError>}
        </Field>
        <Field data-invalid={contentError !== undefined}>
          <FieldLabel htmlFor="entry-content">{t(($) => $.knowledge.entry.content)}</FieldLabel>
          <Textarea
            id="entry-content"
            value={content}
            onChange={(event) => setContent(event.target.value)}
            aria-invalid={contentError !== undefined}
            className="min-h-80"
          />
          {contentError !== undefined && <FieldError>{errorText(contentError)}</FieldError>}
        </Field>
      </FieldGroup>
      {formError !== undefined && <ErrorAlert error={formError} />}
      <div className="flex flex-wrap items-center gap-2">
        <Button type="submit" disabled={save.isPending}>
          {entry === null ? t(($) => $.knowledge.entry.create) : t(($) => $.knowledge.entry.save)}
        </Button>
        {entry !== null && (
          <AlertDialog>
            <AlertDialogTrigger render={<Button type="button" variant="ghost" />}>
              {t(($) => $.knowledge.entry.delete)}
            </AlertDialogTrigger>
            <AlertDialogContent>
              <AlertDialogHeader>
                <AlertDialogTitle>
                  {t(($) => $.knowledge.entry.deleteTitle, { title: entry.title })}
                </AlertDialogTitle>
                <AlertDialogDescription>
                  {t(($) => $.knowledge.entry.deleteBody)}
                </AlertDialogDescription>
              </AlertDialogHeader>
              <AlertDialogFooter>
                <AlertDialogCancel>{t(($) => $.common.actions.cancel)}</AlertDialogCancel>
                <AlertDialogAction
                  variant="destructive"
                  disabled={remove.isPending}
                  onClick={() => remove.mutate(entry, { onSuccess: onDeleted })}
                >
                  {t(($) => $.knowledge.entry.deleteConfirm)}
                </AlertDialogAction>
              </AlertDialogFooter>
            </AlertDialogContent>
          </AlertDialog>
        )}
        {save.isSuccess && (
          <span className="text-sm text-muted-foreground">{t(($) => $.knowledge.entry.saved)}</span>
        )}
      </div>
      {entry !== null && <IndexStatus entry={entry} />}
    </form>
  )
}

/** Whether the entry can be found by meaning, and why not if indexing failed. */
function IndexStatus({ entry }: { entry: EntryView }) {
  const { t } = useTranslation()
  const failureText = useEmbeddingFailureText()
  const reindex = useReindexEntry()
  const index = entry.index
  return (
    <div className="flex flex-col items-start gap-2 text-sm">
      <p role="status" className={cn(index.state === "failed" && "text-destructive")}>
        {t(($) => $.knowledge.index[index.state])}
        {index.state === "failed" && `: ${failureText(index.failure)}`}
      </p>
      {index.state === "failed" && (
        <Button
          type="button"
          variant="outline"
          size="sm"
          disabled={reindex.isPending}
          onClick={() => reindex.mutate(entry)}
        >
          {t(($) => $.knowledge.reindex)}
        </Button>
      )}
      {reindex.isError && <ErrorAlert error={reindex.error} />}
    </div>
  )
}
