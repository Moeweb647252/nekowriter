/** The writing-skill catalogue (design §8: a top-level page, like Knowledge and Prompts).
 *
 * The page lists every active skill. Built-in rows are read-only: they carry a badge and only a
 * preview action. User rows can be created, edited and deleted; deleting one names the books that
 * selected it, because the cascade removes it from those books (design §4). Selection itself lives
 * in Book settings, where a book exists to select for.
 */
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { BookMarked, Pencil, Plus, Trash2 } from "lucide-react"

import type { WritingSkillDto } from "@/api/gen/WritingSkillDto"
import {
  useCreateWritingSkill,
  useDeleteWritingSkill,
  usePatchWritingSkill,
  useWritingSkills,
} from "@/api/hooks/skills"
import { ConfirmDialog } from "@/components/common/ConfirmDialog"
import { EmptyState } from "@/components/common/EmptyState"
import { useErrorToast } from "@/components/common/ErrorToast"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Skeleton } from "@/components/ui/skeleton"
import { Textarea } from "@/components/ui/textarea"

type SkillDraft = {
  name: string
  category: string
  description: string
  body: string
}

const EMPTY_DRAFT: SkillDraft = { name: "", category: "", description: "", body: "" }

type SkillDialog =
  | { mode: "create" }
  | { mode: "edit"; skill: WritingSkillDto }
  | { mode: "preview"; skill: WritingSkillDto }
  | null

/** The writing-skill catalogue page. */
export default function SkillsPage() {
  const { t } = useTranslation()
  const showError = useErrorToast()
  const skills = useWritingSkills()
  const create = useCreateWritingSkill()
  const patch = usePatchWritingSkill()
  const remove = useDeleteWritingSkill()
  const [dialog, setDialog] = useState<SkillDialog>(null)
  const [draft, setDraft] = useState<SkillDraft>(EMPTY_DRAFT)
  const [deleteTarget, setDeleteTarget] = useState<WritingSkillDto | null>(null)

  const openCreate = () => {
    setDraft(EMPTY_DRAFT)
    setDialog({ mode: "create" })
  }
  const openEdit = (skill: WritingSkillDto) => {
    setDraft({
      name: skill.name,
      category: skill.category,
      description: skill.description,
      body: skill.body,
    })
    setDialog({ mode: "edit", skill })
  }

  const save = () => {
    const input = {
      name: draft.name.trim(),
      category: draft.category.trim(),
      description: draft.description.trim(),
      body: draft.body,
    }
    if (dialog?.mode === "create") {
      create.mutate(input, {
        onSuccess: () => setDialog(null),
        onError: showError,
      })
    } else if (dialog?.mode === "edit") {
      patch.mutate(
        { skillId: dialog.skill.id, patch: input },
        { onSuccess: () => setDialog(null), onError: showError },
      )
    }
  }

  const confirmDelete = () => {
    if (deleteTarget === null) {
      return
    }
    remove.mutate(deleteTarget.id, {
      onSuccess: () => setDeleteTarget(null),
      onError: showError,
    })
  }

  const saving = create.isPending || patch.isPending
  const rows = skills.data ?? []

  return (
    <div className="h-full overflow-y-auto p-6">
      <div className="mb-4 flex items-end justify-between">
        <div>
          <h1 className="text-lg font-semibold">{t("skills:title")}</h1>
          <p className="text-sm text-muted-foreground">{t("skills:hint")}</p>
        </div>
        <Button size="sm" className="gap-1.5" onClick={openCreate}>
          <Plus aria-hidden="true" />
          {t("skills:new")}
        </Button>
      </div>

      {skills.isPending ? (
        <Skeleton className="h-48" />
      ) : skills.isError ? (
        <EmptyState title={t("common:error_title")} description={t("common:error_body")} />
      ) : rows.length === 0 ? (
        <EmptyState title={t("skills:empty_title")} description={t("skills:empty_body")} />
      ) : (
        <ul className="space-y-3" aria-label={t("skills:title")}>
          {rows.map((skill) => (
            <li key={skill.id} className="rounded-lg border p-4" data-testid={`skill-${skill.id}`}>
              <div className="flex items-start justify-between gap-3">
                <div className="min-w-0">
                  <div className="flex flex-wrap items-center gap-2">
                    <h2 className="truncate font-medium">{skill.name}</h2>
                    {skill.author === "builtin" ? (
                      <Badge variant="secondary">{t("skills:built_in")}</Badge>
                    ) : null}
                    {skill.category === "" ? null : (
                      <Badge variant="outline">{skill.category}</Badge>
                    )}
                  </div>
                  {skill.description === "" ? null : (
                    <p className="mt-1 text-sm text-muted-foreground">{skill.description}</p>
                  )}
                  <p className="mt-2 line-clamp-3 whitespace-pre-wrap text-sm">{skill.body}</p>
                  {skill.body === "" ? (
                    <p className="mt-2 text-sm text-muted-foreground">
                      {t("skills:empty_body")}
                    </p>
                  ) : null}
                </div>
                <div className="flex shrink-0 items-center gap-1">
                  {skill.author === "builtin" ? (
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => setDialog({ mode: "preview", skill })}
                    >
                      <BookMarked aria-hidden="true" />
                      {t("skills:view")}
                    </Button>
                  ) : (
                    <>
                      <Button
                        variant="ghost"
                        size="icon"
                        aria-label={t("skills:edit")}
                        onClick={() => openEdit(skill)}
                      >
                        <Pencil aria-hidden="true" />
                      </Button>
                      <Button
                        variant="ghost"
                        size="icon"
                        aria-label={t("skills:delete")}
                        onClick={() => setDeleteTarget(skill)}
                      >
                        <Trash2 aria-hidden="true" />
                      </Button>
                    </>
                  )}
                </div>
              </div>
            </li>
          ))}
        </ul>
      )}

      <Dialog
        open={dialog !== null}
        onOpenChange={(open) => {
          if (!open) {
            setDialog(null)
          }
        }}
      >
        <DialogContent className="max-w-2xl">
          <DialogHeader>
            <DialogTitle>
              {dialog?.mode === "create"
                ? t("skills:create_title")
                : dialog?.mode === "edit"
                  ? t("skills:edit_title")
                  : t("skills:preview_title")}
            </DialogTitle>
          </DialogHeader>
          {dialog?.mode === "preview" ? (
            <div className="space-y-3">
              <div>
                <p className="font-medium">{dialog.skill.name}</p>
                <p className="text-sm text-muted-foreground">
                  {dialog.skill.category === ""
                    ? t("skills:no_category")
                    : dialog.skill.category}
                </p>
              </div>
              <pre className="max-h-96 overflow-auto whitespace-pre-wrap rounded-md border bg-muted/30 p-3 text-sm">
                {dialog.skill.body === "" ? t("skills:empty_body") : dialog.skill.body}
              </pre>
              <p className="text-xs text-muted-foreground">{t("skills:builtin_readonly_hint")}</p>
            </div>
          ) : (
            <div className="space-y-3">
              <div className="space-y-1.5">
                <Label htmlFor="skill-name">{t("skills:name")}</Label>
                <Input
                  id="skill-name"
                  value={draft.name}
                  onChange={(event) => setDraft({ ...draft, name: event.target.value })}
                />
              </div>
              <div className="space-y-1.5">
                <Label htmlFor="skill-category">{t("skills:category")}</Label>
                <Input
                  id="skill-category"
                  value={draft.category}
                  onChange={(event) => setDraft({ ...draft, category: event.target.value })}
                />
              </div>
              <div className="space-y-1.5">
                <Label htmlFor="skill-description">{t("skills:description")}</Label>
                <Input
                  id="skill-description"
                  value={draft.description}
                  onChange={(event) => setDraft({ ...draft, description: event.target.value })}
                />
              </div>
              <div className="space-y-1.5">
                <Label htmlFor="skill-body">{t("skills:body")}</Label>
                <Textarea
                  id="skill-body"
                  className="min-h-64"
                  value={draft.body}
                  onChange={(event) => setDraft({ ...draft, body: event.target.value })}
                  placeholder={t("skills:body_placeholder")}
                />
              </div>
            </div>
          )}
          <DialogFooter>
            <Button variant="outline" onClick={() => setDialog(null)}>
              {dialog?.mode === "preview" ? t("common:close") : t("common:cancel")}
            </Button>
            {dialog?.mode === "preview" ? null : (
              <Button disabled={saving || draft.name.trim() === ""} onClick={save}>
                {saving ? t("common:saving") : t("common:save")}
              </Button>
            )}
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <ConfirmDialog
        open={deleteTarget !== null}
        onOpenChange={(open) => {
          if (!open) {
            setDeleteTarget(null)
          }
        }}
        title={t("skills:delete_title", { name: deleteTarget?.name ?? "" })}
        description={t("skills:delete_body")}
        details={
          deleteTarget !== null && deleteTarget.used_by_books.length > 0 ? (
            <div>
              <p>{t("skills:delete_used_by")}</p>
              <ul className="list-inside list-disc">
                {deleteTarget.used_by_books.map((book) => (
                  <li key={book.id}>{book.title}</li>
                ))}
              </ul>
            </div>
          ) : undefined
        }
        confirmLabel={t("common:delete")}
        pending={remove.isPending}
        onConfirm={confirmDelete}
      />
    </div>
  )
}
