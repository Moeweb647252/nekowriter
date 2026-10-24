import { useTranslation } from "react-i18next"

/** The book library (PLAN M1.4 fills in cards and the new-book dialog). */
export default function LibraryPage() {
  const { t } = useTranslation()
  return (
    <div className="mx-auto max-w-3xl p-6">
      <h1 className="text-2xl font-semibold">{t("library:title")}</h1>
      <p className="mt-1 text-muted-foreground">{t("library:empty_body")}</p>
    </div>
  )
}
