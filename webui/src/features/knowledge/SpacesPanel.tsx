import { PlusIcon } from "lucide-react"
import { type SubmitEvent, useState } from "react"
import { useTranslation } from "react-i18next"

import { errorForField, errorForForm } from "@/api/fieldErrors"
import type { KnowledgeSpaceId } from "@/api/generated/KnowledgeSpaceId"
import type { SpaceView } from "@/api/generated/SpaceView"
import { useCreateSpace, useDeleteSpace, useRenameSpace } from "@/api/queries/knowledge"
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
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog"
import { Field, FieldError, FieldLabel } from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import { useErrorText } from "@/i18n/errors"
import { cn } from "@/lib/utils"

/** The spaces, with "all entries" first, and actions for the selected one. */
export function SpacesPanel({
  spaces,
  selected,
  onSelect,
}: {
  spaces: readonly SpaceView[]
  selected: KnowledgeSpaceId | null
  onSelect: (space: KnowledgeSpaceId | null) => void
}) {
  const { t } = useTranslation()
  const current = spaces.find((space) => space.id === selected)
  const item = (active: boolean) =>
    cn(
      "flex w-full items-baseline justify-between gap-2 rounded-md px-2 py-1.5 text-left text-sm text-muted-foreground hover:text-foreground",
      active && "bg-muted text-foreground",
    )

  return (
    <nav aria-label={t(($) => $.knowledge.spaces)} className="flex flex-col gap-3">
      <h2 className="px-2 text-xs font-medium text-muted-foreground uppercase">
        {t(($) => $.knowledge.spaces)}
      </h2>
      <ul className="flex flex-col">
        <li>
          <button
            type="button"
            aria-current={selected === null ? "true" : undefined}
            onClick={() => onSelect(null)}
            className={item(selected === null)}
          >
            {t(($) => $.knowledge.allEntries)}
          </button>
        </li>
        {spaces.map((space) => (
          <li key={space.id}>
            <button
              type="button"
              aria-current={space.id === selected ? "true" : undefined}
              onClick={() => onSelect(space.id)}
              className={item(space.id === selected)}
            >
              <span className="truncate">{space.name}</span>
              <span className="shrink-0 text-xs">
                {t(($) => $.knowledge.entryCount, { count: space.entryCount })}
              </span>
            </button>
          </li>
        ))}
      </ul>
      <div className="flex flex-wrap gap-2">
        <SpaceNameDialog space={null} onCreated={onSelect} />
        {current !== undefined && (
          <>
            <SpaceNameDialog space={current} onCreated={onSelect} />
            <DeleteSpace space={current} onDeleted={() => onSelect(null)} />
          </>
        )}
      </div>
    </nav>
  )
}

/** Names a new space, or renames `space`. */
function SpaceNameDialog({
  space,
  onCreated,
}: {
  space: SpaceView | null
  onCreated: (space: KnowledgeSpaceId) => void
}) {
  const { t } = useTranslation()
  const errorText = useErrorText()
  const create = useCreateSpace()
  const rename = useRenameSpace()
  const [open, setOpen] = useState(false)
  const [name, setName] = useState(space?.name ?? "")
  const save = space === null ? create : rename

  const submit = (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    const done = {
      onSuccess: (saved: SpaceView) => {
        setOpen(false)
        if (space === null) onCreated(saved.id)
      },
    }
    if (space === null) create.mutate(name, done)
    else rename.mutate({ space, name }, done)
  }

  const nameError = errorForField(save.error, "space_name")
  const formError = errorForForm(save.error, ["space_name"])
  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        setOpen(next)
        if (next) {
          setName(space?.name ?? "")
          save.reset()
        }
      }}
    >
      <DialogTrigger render={<Button type="button" variant="outline" size="sm" />}>
        {space === null ? (
          <>
            <PlusIcon />
            {t(($) => $.knowledge.newSpace)}
          </>
        ) : (
          t(($) => $.knowledge.rename)
        )}
      </DialogTrigger>
      <DialogContent>
        <form onSubmit={submit} className="flex flex-col gap-4">
          <DialogHeader>
            <DialogTitle>
              {space === null
                ? t(($) => $.knowledge.newSpace)
                : t(($) => $.knowledge.renameTitle, { name: space.name })}
            </DialogTitle>
          </DialogHeader>
          <Field data-invalid={nameError !== undefined}>
            <FieldLabel htmlFor="space-name">{t(($) => $.knowledge.spaceName)}</FieldLabel>
            <Input
              id="space-name"
              value={name}
              onChange={(event) => setName(event.target.value)}
              aria-invalid={nameError !== undefined}
            />
            {nameError !== undefined && <FieldError>{errorText(nameError)}</FieldError>}
          </Field>
          {formError !== undefined && <ErrorAlert error={formError} />}
          <DialogFooter>
            <Button type="submit" disabled={save.isPending}>
              {space === null ? t(($) => $.knowledge.createSpace) : t(($) => $.knowledge.saveName)}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}

function DeleteSpace({ space, onDeleted }: { space: SpaceView; onDeleted: () => void }) {
  const { t } = useTranslation()
  const remove = useDeleteSpace()
  return (
    <AlertDialog onOpenChange={(open) => open && remove.reset()}>
      <AlertDialogTrigger render={<Button type="button" variant="ghost" size="sm" />}>
        {t(($) => $.knowledge.deleteSpace)}
      </AlertDialogTrigger>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>
            {t(($) => $.knowledge.deleteSpaceTitle, { name: space.name })}
          </AlertDialogTitle>
          <AlertDialogDescription>{t(($) => $.knowledge.deleteSpaceBody)}</AlertDialogDescription>
        </AlertDialogHeader>
        {remove.isError && <ErrorAlert error={remove.error} />}
        <AlertDialogFooter>
          <AlertDialogCancel>{t(($) => $.common.actions.cancel)}</AlertDialogCancel>
          <AlertDialogAction
            variant="destructive"
            disabled={remove.isPending}
            onClick={() => remove.mutate(space, { onSuccess: onDeleted })}
          >
            {t(($) => $.knowledge.deleteSpace)}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
