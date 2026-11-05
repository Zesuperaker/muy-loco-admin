import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import { Link, Route, Routes, useParams } from "react-router";
import { ApiError, api, getToken, onUnauthorized, setToken } from "./api";
import { Dashboard } from "./components/Dashboard";
import { Layout } from "./components/Layout";
import { Login } from "./components/Login";
import { ModelList } from "./components/ModelList";
import { RecordPage } from "./components/RecordPage";
import { ErrorBanner, Spinner } from "./components/ui";
import { paths } from "./paths";
import type { AdminConfig, AdminUser, ModelMeta } from "./types";

function NotFound() {
  return (
    <div className="flex flex-col gap-2">
      <h1 className="text-2xl font-semibold">Not found</h1>
      <Link to={paths.dashboard()} className="text-indigo-600 hover:underline">
        Back to the dashboard
      </Link>
    </div>
  );
}

function useTitle(title: string) {
  useEffect(() => {
    document.title = title;
  }, [title]);
}

function ModelRoute({
  models,
  view,
  title,
}: {
  models: ModelMeta[];
  view: "list" | "new" | "record";
  title: string;
}) {
  const params = useParams();
  const model = models.find((m) => m.name === params.model);
  useTitle(model ? `${model.label} · ${title}` : title);
  if (!model) return <NotFound />;
  switch (view) {
    case "list":
      return <ModelList key={model.name} model={model} models={models} />;
    case "new":
      return <RecordPage key={`${model.name}:new`} model={model} models={models} />;
    case "record":
      return (
        <RecordPage
          key={`${model.name}:${params.key}`}
          model={model}
          models={models}
          recordKey={params.key}
        />
      );
  }
}

function Shell({
  config,
  user,
  onSignOut,
}: {
  config: AdminConfig;
  user: AdminUser;
  onSignOut: () => void;
}) {
  const schema = useQuery({ queryKey: ["schema"], queryFn: api.schema });
  useTitle(config.title);
  const models = schema.data?.models ?? [];
  return (
    <Layout title={config.title} user={user} models={models} onSignOut={onSignOut}>
      {schema.isPending ? (
        <Spinner />
      ) : schema.error ? (
        <ErrorBanner error={schema.error} />
      ) : (
        <Routes>
          <Route path="/" element={<Dashboard models={models} />} />
          <Route
            path="/models/:model"
            element={<ModelRoute models={models} view="list" title={config.title} />}
          />
          <Route
            path="/models/:model/new"
            element={<ModelRoute models={models} view="new" title={config.title} />}
          />
          <Route
            path="/models/:model/records/:key"
            element={<ModelRoute models={models} view="record" title={config.title} />}
          />
          <Route path="*" element={<NotFound />} />
        </Routes>
      )}
    </Layout>
  );
}

export function App() {
  const queryClient = useQueryClient();
  const config = useQuery({ queryKey: ["config"], queryFn: api.config, staleTime: Infinity });
  const me = useQuery({
    queryKey: ["me"],
    queryFn: api.me,
    enabled: Boolean(config.data),
    retry: false,
  });

  useEffect(
    () =>
      onUnauthorized(() => {
        if (getToken()) {
          setToken(null);
          void queryClient.invalidateQueries({ queryKey: ["me"] });
        }
      }),
    [queryClient],
  );

  const restart = () => {
    void queryClient.resetQueries({ predicate: (q) => q.queryKey[0] !== "config" });
  };
  const signOut = () => {
    setToken(null);
    restart();
  };

  if (config.isPending) return <Spinner />;
  if (config.error) {
    return (
      <div className="p-6">
        <ErrorBanner error={config.error} />
      </div>
    );
  }
  if (me.isPending) return <Spinner />;
  if (me.error) {
    const status = me.error instanceof ApiError ? me.error.status : 0;
    if (status === 401 || status === 403) {
      return <Login config={config.data} forbidden={status === 403} onSignedIn={restart} />;
    }
    return (
      <div className="p-6">
        <ErrorBanner error={me.error} />
      </div>
    );
  }
  return <Shell config={config.data} user={me.data} onSignOut={signOut} />;
}
