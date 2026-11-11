import { useQuery } from "@tanstack/react-query";
import { api } from "../api";
import { Link } from "react-router";
import { countNote, formatCount } from "../format";
import { paths } from "../paths";
import type { ModelMeta } from "../types";
import { Card, ErrorBanner } from "./ui";

/** How many records each model the user may view has, linking to its list. */
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
        {models.map((model) => {
          const count = stats.data?.counts[model.name] ?? { kind: "exact" as const, value: 0 };
          return (
            <Link key={model.name} to={paths.list(model.name)} className="group">
              <Card className="p-5 transition group-hover:border-indigo-300">
                <div className="text-sm text-slate-500">{model.label}</div>
                <div
                  className="mt-1 text-3xl font-semibold"
                  data-testid={`count-${model.name}`}
                  title={stats.data ? countNote(count) : undefined}
                >
                  {stats.data ? formatCount(count) : "…"}
                </div>
              </Card>
            </Link>
          );
        })}
      </div>
    </div>
  );
}
