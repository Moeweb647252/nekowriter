/** The shared empty state (PROJECT §8.13: every list says what it is and how to fill it). */
import { Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyTitle } from "@/components/ui/empty"

export type EmptyStateProps = {
  title: string
  description?: string
  children?: React.ReactNode
  className?: string
}

export function EmptyState({ title, description, children, className }: EmptyStateProps) {
  return (
    <Empty className={className}>
      <EmptyHeader>
        <EmptyTitle>{title}</EmptyTitle>
        {description === undefined ? null : (
          <EmptyDescription>{description}</EmptyDescription>
        )}
      </EmptyHeader>
      {children === undefined ? null : <EmptyContent>{children}</EmptyContent>}
    </Empty>
  )
}
