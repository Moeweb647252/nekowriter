import { useTranslation } from "react-i18next"

import { useShutdown } from "@/api/queries/system"
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
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"

/**
 * Exiting the local service. Once it stops, the event stream reports it and the app
 * shows how to start again.
 */
export function ServiceCard() {
  const { t } = useTranslation()
  const shutdown = useShutdown()

  return (
    <Card>
      <CardHeader>
        <CardTitle>{t(($) => $.settings.service.title)}</CardTitle>
        <CardDescription>{t(($) => $.settings.service.body)}</CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col items-start gap-4">
        {shutdown.isError && <ErrorAlert error={shutdown.error} />}
        <AlertDialog>
          <AlertDialogTrigger render={<Button variant="destructive" />}>
            {t(($) => $.settings.service.exit)}
          </AlertDialogTrigger>
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>{t(($) => $.settings.service.confirmTitle)}</AlertDialogTitle>
              <AlertDialogDescription>
                {t(($) => $.settings.service.confirmBody)}
              </AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel>{t(($) => $.common.actions.cancel)}</AlertDialogCancel>
              <AlertDialogAction
                variant="destructive"
                disabled={shutdown.isPending}
                onClick={() => shutdown.mutate()}
              >
                {t(($) => $.settings.service.confirm)}
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      </CardContent>
    </Card>
  )
}
