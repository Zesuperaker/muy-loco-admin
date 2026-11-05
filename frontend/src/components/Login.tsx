import { type FormEvent, useState } from "react";
import { login } from "../api";
import type { AdminConfig } from "../types";
import { Button, Card, ErrorBanner } from "./ui";

export function Login({
  config,
  forbidden,
  onSignedIn,
}: {
  config: AdminConfig;
  forbidden: boolean;
  onSignedIn: () => void;
}) {
  const [identifier, setIdentifier] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<unknown>(null);
  const [busy, setBusy] = useState(false);
  const method = config.login;

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (method.type !== "form") return;
    setBusy(true);
    setError(null);
    try {
      await login(method, identifier, password);
      onSignedIn();
    } catch (err) {
      setError(err);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="flex min-h-screen items-center justify-center p-4">
      <Card className="w-full max-w-sm p-6">
        <h1 className="mb-1 text-xl font-semibold">{config.title}</h1>
        <p className="mb-5 text-sm text-slate-500">
          {forbidden ? "Your account does not have admin access." : "Sign in to continue."}
        </p>
        {method.type === "form" && (
          <form onSubmit={submit} className="flex flex-col gap-4">
            <ErrorBanner error={error} />
            <label className="flex flex-col gap-1 text-sm font-medium">
              {method.identifier_label}
              <input
                className="input"
                autoComplete="username"
                required
                value={identifier}
                onChange={(e) => setIdentifier(e.target.value)}
              />
            </label>
            <label className="flex flex-col gap-1 text-sm font-medium">
              Password
              <input
                className="input"
                type="password"
                autoComplete="current-password"
                required
                value={password}
                onChange={(e) => setPassword(e.target.value)}
              />
            </label>
            <Button type="submit" variant="primary" disabled={busy} className="justify-center">
              {busy ? "Signing in…" : "Sign in"}
            </Button>
          </form>
        )}
        {method.type === "redirect" && (
          <a className="font-medium text-indigo-600 hover:underline" href={method.url}>
            Continue to sign in
          </a>
        )}
        {method.type === "none" && (
          <Button onClick={onSignedIn}>Retry</Button>
        )}
      </Card>
    </div>
  );
}
