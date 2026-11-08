import type { ReactNode } from "react";
import { Link, NavLink } from "react-router";
import { paths } from "../paths";
import type { AdminUser, ModelMeta } from "../types";
import { Button } from "./ui";

function navClass({ isActive }: { isActive: boolean }): string {
  return `rounded-md px-3 py-1.5 text-sm ${
    isActive ? "bg-indigo-50 font-medium text-indigo-700" : "text-slate-600 hover:bg-slate-100"
  }`;
}

export function Layout({
  title,
  siteUrl,
  user,
  models,
  onSignOut,
  children,
}: {
  title: string;
  siteUrl: string | null;
  user: AdminUser;
  models: ModelMeta[];
  onSignOut: () => void;
  children: ReactNode;
}) {
  return (
    <div className="flex min-h-screen flex-col md:flex-row">
      <aside className="border-b border-slate-200 bg-white md:w-60 md:shrink-0 md:border-r md:border-b-0">
        <div className="px-5 py-4">
          <Link to={paths.dashboard()} className="text-lg font-semibold text-slate-900">
            {title}
          </Link>
        </div>
        <nav aria-label="Models" className="flex flex-row flex-wrap gap-1 px-3 pb-3 md:flex-col">
          {models.map((model) => (
            <NavLink
              key={model.name}
              to={paths.list(model.name)}
              className={navClass}
            >
              {model.label}
            </NavLink>
          ))}
        </nav>
        {user.can_manage_roles && (
          <nav
            aria-label="Settings"
            className="flex flex-row flex-wrap gap-1 border-t border-slate-200 px-3 py-3 md:flex-col"
          >
            <NavLink
              to={paths.roles()}
              className={navClass}
            >
              Roles
            </NavLink>
          </nav>
        )}
      </aside>
      <div className="flex min-w-0 flex-1 flex-col">
        <header className="flex items-center justify-end gap-3 border-b border-slate-200 bg-white px-6 py-3 text-sm">
          {siteUrl && (
            <a
              href={siteUrl}
              className="font-medium text-indigo-600 hover:underline"
            >
              Visit site
            </a>
          )}
          <span className="text-slate-500">{user.name}</span>
          {user.role && (
            <span
              title="Your role"
              className="rounded-full bg-slate-100 px-2 py-0.5 text-xs font-medium text-slate-600"
            >
              {user.role.name}
            </span>
          )}
          <Button variant="ghost" onClick={onSignOut}>
            Sign out
          </Button>
        </header>
        <main className="flex-1 p-4 md:p-6">{children}</main>
      </div>
    </div>
  );
}
