import { useQuery } from "@tanstack/react-query";
import { api } from "../api";
import { Link, paths } from "../router";
import type { ModelMeta } from "../types";
import { Card, ErrorBanner } from "./ui";

export function Dashboard({ models }: { models: ModelMeta[] }) {
  const stats = useQuery({ queryKey: ["stats"], queryFn: api.stats });
  return (
    <div className="flex flex-col gap-4">
      <h1 className="text-2xl font-semibold">Dashboard</h1>
      <ErrorBanner error={stats.error} />
      {models.length === 0 && (
        <p className="text-sm text-slate-500">No models are available to you.</p>
      )}
      <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-3">
        {models.map((model) => (
          <Link key={model.name} to={paths.list(model.name)} className="group">
            <Card className="p-5 transition group-hover:border-indigo-300">
              <div className="text-sm text-slate-500">{model.label}</div>
              <div className="mt-1 text-3xl font-semibold" data-testid={`count-${model.name}`}>
                {stats.data ? (stats.data.counts[model.name] ?? 0) : "…"}
              </div>
            </Card>
          </Link>
        ))}
      </div>
    </div>
  );
}
