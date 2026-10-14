import { useTranslation } from "react-i18next"

import { Button } from "@/components/ui/button"
import { Field, FieldDescription, FieldError, FieldLabel } from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import { useErrorText } from "@/i18n/errors"

/**
 * A write-only API key (§4 invariant 17): a new key replaces the stored one, which is
 * never shown; the stored key can be marked for removal instead.
 */
export function ApiKeyField({
  id,
  stored,
  value,
  onChange,
  clear,
  onClear,
  error,
}: {
  id: string
  /** Whether a key is stored. */
  stored: boolean
  value: string
  onChange: (key: string) => void
  /** Whether saving removes the stored key. */
  clear: boolean
  onClear: (clear: boolean) => void
  error: unknown
}) {
  const { t } = useTranslation()
  const errorText = useErrorText()
  return (
    <Field data-invalid={error !== undefined}>
      <FieldLabel htmlFor={id}>{t(($) => $.settings.model.apiKey)}</FieldLabel>
      <Input
        id={id}
        type="password"
        value={value}
        onChange={(event) => onChange(event.target.value)}
        aria-invalid={error !== undefined}
        autoComplete="new-password"
      />
      <FieldDescription>
        {clear && value === ""
          ? t(($) => $.settings.model.keyWillClear)
          : stored
            ? t(($) => $.settings.model.apiKeyStored)
            : t(($) => $.settings.model.apiKeyNone)}
      </FieldDescription>
      {stored && (
        <div>
          <Button
            type="button"
            variant="link"
            className="h-auto p-0"
            onClick={() => onClear(!clear)}
          >
            {clear ? t(($) => $.settings.model.keepKey) : t(($) => $.settings.model.clearKey)}
          </Button>
        </div>
      )}
      {error !== undefined && <FieldError>{errorText(error)}</FieldError>}
    </Field>
  )
}
