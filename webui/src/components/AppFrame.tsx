import { type ReactNode } from "react"
import {
  BookOpenText,
  ChartNoAxesCombined,
  Database,
  FileText,
  Settings2,
  Trash2,
} from "lucide-react"
import { Link, useLocation } from "react-router"

import { ServiceStoppedNotice } from "@/components/ServiceStoppedNotice"
import { WorkspaceControls } from "@/components/WorkspaceControls"
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarInset,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarProvider,
  SidebarTrigger,
} from "@/components/ui/sidebar"

type NavigationItem = {
  label: string
  to: string
  icon: typeof BookOpenText
}

const navigation: NavigationItem[] = [
  { label: "作品库", to: "/works", icon: BookOpenText },
  { label: "任务概览", to: "/dashboard", icon: ChartNoAxesCombined },
  { label: "知识库", to: "/knowledge", icon: Database },
  { label: "提示词模板", to: "/templates", icon: FileText },
  { label: "系统设置", to: "/settings", icon: Settings2 },
  { label: "回收站", to: "/recycle", icon: Trash2 },
]

function isCurrentPath(pathname: string, destination: string) {
  return destination === "/works"
    ? pathname === destination
    : pathname === destination || pathname.startsWith(`${destination}/`)
}

/** 应用级导航只放跨作品功能；写作工作台使用自己的章节目录，避免两层侧栏竞争。 */
export function AppFrame({
  title,
  description,
  actions,
  children,
}: {
  title: string
  description: string
  actions?: ReactNode
  children: ReactNode
}) {
  const location = useLocation()

  return (
    <SidebarProvider>
      <a
        className="sr-only fixed top-3 left-3 z-50 rounded-md bg-primary px-3 py-2 text-sm font-medium text-primary-foreground focus:not-sr-only focus-visible:ring-3 focus-visible:ring-ring focus-visible:outline-none"
        href="#main-content"
        onClick={(event) => {
          event.preventDefault()
          document.getElementById("main-content")?.focus()
        }}
      >
        跳到主要内容
      </a>
      <Sidebar collapsible="icon">
        <SidebarHeader>
          <SidebarMenu>
            <SidebarMenuItem>
              <SidebarMenuButton
                render={<Link to="/works" />}
                tooltip="NekoWriter · 作品库"
                size="lg"
              >
                <BookOpenText />
                <span>NekoWriter</span>
              </SidebarMenuButton>
            </SidebarMenuItem>
          </SidebarMenu>
        </SidebarHeader>
        <SidebarContent>
          <SidebarGroup>
            <SidebarGroupLabel>工作区</SidebarGroupLabel>
            <SidebarGroupContent>
              <SidebarMenu>
                {navigation.map((item) => {
                  const Icon = item.icon
                  return (
                    <SidebarMenuItem key={item.to}>
                      <SidebarMenuButton
                        render={<Link to={item.to} />}
                        isActive={isCurrentPath(location.pathname, item.to)}
                        tooltip={item.label}
                      >
                        <Icon />
                        <span>{item.label}</span>
                      </SidebarMenuButton>
                    </SidebarMenuItem>
                  )
                })}
              </SidebarMenu>
            </SidebarGroupContent>
          </SidebarGroup>
        </SidebarContent>
        <SidebarFooter>
          <p className="px-3 text-xs leading-5 text-sidebar-foreground/60 group-data-[collapsible=icon]:hidden">
            所有作品与运行记录保存在本机。
          </p>
        </SidebarFooter>
      </Sidebar>
      <SidebarInset>
        <ServiceStoppedNotice />
        <header className="flex min-h-14 shrink-0 items-center gap-3 border-b bg-background/90 px-4 backdrop-blur sm:px-6">
          <SidebarTrigger />
          <div className="min-w-0 flex-1">
            <h1 className="truncate font-heading text-base font-medium">
              {title}
            </h1>
            <p className="hidden truncate text-xs text-muted-foreground sm:block">
              {description}
            </p>
          </div>
          <div className="flex shrink-0 items-center gap-2">
            {actions}
            <WorkspaceControls />
          </div>
        </header>
        <main
          className="min-h-0 flex-1 overflow-auto"
          id="main-content"
          tabIndex={-1}
        >
          {children}
        </main>
      </SidebarInset>
    </SidebarProvider>
  )
}
