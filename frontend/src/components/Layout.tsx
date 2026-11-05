import type { ReactNode } from "react";
import { Link } from "../router";
import type { AdminUser, ModelMeta } from "../types";
import { Button } from "./ui";

export function Layout({
  title,
  user,
  models,
  current,
  onSignOut,
  children,
}: {
  title: string;
  user: AdminUser;
  models: ModelMeta[];
  current?: string;
  onSignOut: () => void;
  children: ReactNode;
}) {
  return (
    <div className="flex min-h-screen flex-col md:flex-row">
      <aside className="border-b border-slate-200 bg-white md:w-60 md:shrink-0 md:border-r md:border-b-0 dark:border-slate-800 dark:bg-slate-900">
        <div className="px-5 py-4">
          <Link to="/" className="text-lg font-semibold text-slate-900 dark:text-white">
            {title}
          </Link>
        </div>
        <nav aria-label="Models" className="flex flex-row flex-wrap gap-1 px-3 pb-3 md:flex-col">
          {models.map((model) => (
            <Link
              key={model.name}
              to={`/models/${encodeURIComponent(model.name)}`}
              aria-current={current === model.name ? "page" : undefined}
              className={`rounded-md px-3 py-1.5 text-sm ${
                current === model.name
                  ? "bg-indigo-50 font-medium text-indigo-700 dark:bg-indigo-950 dark:text-indigo-300"
                  : "text-slate-600 hover:bg-slate-100 dark:text-slate-300 dark:hover:bg-slate-800"
              }`}
            >
              {model.label}
            </Link>
          ))}
        </nav>
      </aside>
      <div className="flex min-w-0 flex-1 flex-col">
        <header className="flex items-center justify-end gap-3 border-b border-slate-200 bg-white px-6 py-3 text-sm dark:border-slate-800 dark:bg-slate-900">
          <span className="text-slate-500">{user.name}</span>
          <Button variant="ghost" onClick={onSignOut}>
            Sign out
          </Button>
        </header>
        <main className="flex-1 p-4 md:p-6">{children}</main>
      </div>
    </div>
  );
}
