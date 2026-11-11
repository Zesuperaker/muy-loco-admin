import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { type FormEvent, useState } from "react";
import { Link, useNavigate } from "react-router";
import { ApiError, api } from "../api";
import { paths } from "../paths";
import {
  OPERATIONS,
  OPERATION_LABELS,
  grantsAnything,
  offers,
  plainIdWarnings,
  summary,
  toggleAction,
  toggleOperation,
  userWarnings,
} from "../roles";
import type { ModelMeta, Permissions, RoleWithUsers } from "../types";
import { Button, Card, ErrorBanner, Spinner, useToast } from "./ui";

const linkButton =
  "inline-flex items-center rounded-md bg-indigo-600 px-3 py-1.5 text-sm font-medium text-white shadow-sm hover:bg-indigo-500";

export function RoleList({ models }: { models: ModelMeta[] }) {
  const roles = useQuery({ queryKey: ["roles"], queryFn: api.roles });
  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-2xl font-semibold">Roles</h1>
          <p className="text-sm text-slate-500">
            Admins without a role have full access. Give a user a role on their record to limit
            what they can do.
          </p>
        </div>
        <Link to={paths.newRole()} className={linkButton}>
          Add role
        </Link>
      </div>
      {roles.isPending ? (
        <Spinner />
      ) : roles.error ? (
        <ErrorBanner error={roles.error} />
      ) : roles.data.roles.length === 0 ? (
        <p className="text-sm text-slate-500">No roles yet.</p>
      ) : (
        <Card className="overflow-x-auto">
          <table className="min-w-full divide-y divide-slate-200 text-sm">
            <thead className="bg-slate-50 text-left text-xs font-semibold tracking-wide text-slate-500 uppercase">
              <tr>
                <th className="px-4 py-2">Name</th>
                <th className="px-4 py-2">Can view</th>
                <th className="px-4 py-2">Users</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-slate-100">
              {roles.data.roles.map((role) => (
                <tr key={role.id}>
                  <td className="px-4 py-2">
                    <Link
                      to={paths.role(role.id)}
                      className="font-medium text-indigo-600 hover:underline"
                    >
                      {role.name}
                    </Link>
                  </td>
                  <td className="px-4 py-2">{summary(role.permissions, models)}</td>
                  <td className="px-4 py-2">{role.users}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </Card>
      )}
    </div>
  );
}

function PermissionTable({
  models,
  permissions,
  disabled,
  onChange,
}: {
  models: ModelMeta[];
  permissions: Permissions;
  disabled: boolean;
  onChange: (next: Permissions) => void;
}) {
  return (
    <Card className="overflow-x-auto">
      <table className="min-w-full divide-y divide-slate-200 text-sm">
        <thead className="bg-slate-50 text-left text-xs font-semibold tracking-wide text-slate-500 uppercase">
          <tr>
            <th className="px-4 py-2">Model</th>
            {OPERATIONS.map((op) => (
              <th key={op} className="px-4 py-2 text-center">
                {OPERATION_LABELS[op]}
              </th>
            ))}
            <th className="px-4 py-2">Actions</th>
          </tr>
        </thead>
        <tbody className="divide-y divide-slate-100">
          {models.map((model) => {
            const grant = permissions[model.name];
            return (
              <tr key={model.name}>
                <th scope="row" className="px-4 py-2 text-left font-medium">
                  {model.label}
                </th>
                {OPERATIONS.map((op) => (
                  <td key={op} className="px-4 py-2 text-center">
                    <input
                      type="checkbox"
                      aria-label={`${model.label}: ${OPERATION_LABELS[op]}`}
                      className="h-4 w-4 rounded border-slate-300 text-indigo-600"
                      disabled={disabled || !offers(model, op)}
                      title={offers(model, op) ? undefined : "Not allowed for this model"}
                      checked={op === "view" ? grantsAnything(grant) : Boolean(grant?.[op])}
                      onChange={(e) =>
                        onChange(toggleOperation(permissions, model.name, op, e.target.checked))
                      }
                    />
                  </td>
                ))}
                <td className="px-4 py-2">
                  <div className="flex flex-wrap gap-3">
                    {model.actions.map((action) => (
                      <label key={action.name} className="inline-flex items-center gap-1.5">
                        <input
                          type="checkbox"
                          aria-label={`${model.label}: ${action.label}`}
                          className="h-4 w-4 rounded border-slate-300 text-indigo-600"
                          disabled={disabled}
                          checked={grant?.actions.includes(action.name) ?? false}
                          onChange={(e) =>
                            onChange(
                              toggleAction(permissions, model.name, action.name, e.target.checked),
                            )
                          }
                        />
                        {action.label}
                      </label>
                    ))}
                  </div>
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </Card>
  );
}

function RoleForm({ models, role }: { models: ModelMeta[]; role?: RoleWithUsers }) {
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const { notify } = useToast();
  const [name, setName] = useState(role?.name ?? "");
  const [permissions, setPermissions] = useState<Permissions>(role?.permissions ?? {});
  const [errors, setErrors] = useState<Record<string, string[]>>({});

  const save = useMutation({
    mutationFn: () => {
      const input = { name, permissions };
      return role ? api.updateRole(role.id, input) : api.createRole(input);
    },
    onSuccess: (res) => {
      setErrors({});
      void queryClient.invalidateQueries({ queryKey: ["roles"] });
      queryClient.setQueryData(["role", res.role.id], res);
      if (role) {
        notify("Saved");
        setPermissions(res.role.permissions);
        setName(res.role.name);
      } else {
        notify(`Created ${res.role.name}`);
        navigate(paths.role(res.role.id), { replace: true });
      }
    },
    onError: (err) => {
      if (err instanceof ApiError) setErrors(err.fields);
    },
  });

  const remove = useMutation({
    mutationFn: () => api.deleteRole(role?.id ?? 0),
    onSuccess: () => {
      notify("Deleted");
      void queryClient.invalidateQueries({ queryKey: ["roles"] });
      navigate(paths.roles(), { replace: true });
    },
    onError: (err) => notify(err.message, "error"),
  });

  const submit = (event: FormEvent) => {
    event.preventDefault();
    save.mutate();
  };

  const warnings = [...userWarnings(permissions, models), ...plainIdWarnings(permissions, models)];
  const busy = save.isPending || remove.isPending;

  return (
    <form onSubmit={submit} className="flex flex-col gap-4" noValidate>
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <Link to={paths.roles()} className="text-sm text-indigo-600 hover:underline">
            ← Roles
          </Link>
          <h1 className="text-2xl font-semibold">{role ? role.name : "New role"}</h1>
          {role && (
            <p className="text-sm text-slate-500">
              {role.users === 1 ? "1 user has" : `${role.users} users have`} this role
            </p>
          )}
        </div>
        <div className="flex gap-2">
          {role && (
            <Button
              variant="danger"
              disabled={busy || role.users > 0}
              title={role.users > 0 ? "Give its users another role first" : undefined}
              onClick={() => {
                if (window.confirm(`Delete the role ${role.name}?`)) remove.mutate();
              }}
            >
              Delete
            </Button>
          )}
          <Button type="submit" variant="primary" disabled={busy}>
            {save.isPending ? "Saving…" : role ? "Save" : "Create"}
          </Button>
        </div>
      </div>

      {save.error && <ErrorBanner error={save.error} />}

      <Card className="grid gap-2 px-5 py-4 sm:grid-cols-[12rem_1fr] sm:items-start">
        <label htmlFor="role-name" className="pt-1.5 text-sm font-medium text-slate-700">
          Name
        </label>
        <div className="flex flex-col gap-1">
          <input
            id="role-name"
            className="input"
            maxLength={100}
            value={name}
            disabled={busy}
            onChange={(e) => setName(e.target.value)}
          />
          {(errors.name ?? []).map((message) => (
            <p key={message} className="text-sm text-rose-600">
              {message}
            </p>
          ))}
        </div>
      </Card>

      <div className="flex flex-col gap-2">
        <h2 className="text-sm font-semibold text-slate-500 uppercase">Permissions</h2>
        <p className="text-sm text-slate-500">
          Models without View are hidden from this role. Any other permission includes View.
        </p>
        {(errors.permissions ?? []).map((message) => (
          <p key={message} className="text-sm text-rose-600">
            {message}
          </p>
        ))}
        <PermissionTable
          models={models}
          permissions={permissions}
          disabled={busy}
          onChange={setPermissions}
        />
      </div>

      {warnings.length > 0 && (
        <ul
          aria-label="Warnings"
          className="list-disc rounded-md border border-amber-200 bg-amber-50 py-2 pr-4 pl-8 text-sm text-amber-800"
        >
          {warnings.map((warning) => (
            <li key={warning}>{warning}</li>
          ))}
        </ul>
      )}
    </form>
  );
}

export function RolePage({ models, id }: { models: ModelMeta[]; id?: string }) {
  const roleId = Number(id);
  const role = useQuery({
    queryKey: ["role", roleId],
    queryFn: () => api.role(roleId),
    enabled: id !== undefined,
  });
  if (id === undefined) return <RoleForm models={models} />;
  if (role.isPending) return <Spinner />;
  if (role.error) return <ErrorBanner error={role.error} />;
  return <RoleForm key={role.data.role.id} models={models} role={role.data.role} />;
}
