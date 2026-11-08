import type { Grant, ModelMeta, Permissions } from "./types";

/** The operations a role can be granted on a model, in table order. */
export const OPERATIONS = ["view", "create", "update", "delete", "export"] as const;
export type Operation = (typeof OPERATIONS)[number];

export const OPERATION_LABELS: Record<Operation, string> = {
  view: "View",
  create: "Create",
  update: "Edit",
  delete: "Delete",
  export: "Export",
};

export function emptyGrant(): Grant {
  return { view: false, create: false, update: false, delete: false, export: false, actions: [] };
}

export function grantsAnything(grant: Grant | undefined): boolean {
  if (!grant) return false;
  return OPERATIONS.some((op) => grant[op]) || grant.actions.length > 0;
}

/** Whether the model offers `op` at all (its `ModelAdmin` settings). */
export function offers(model: ModelMeta, op: Operation): boolean {
  return model.capabilities[op];
}

/**
 * Turn `op` on or off for a model. Anything granted implies view, and
 * removing view removes the rest.
 */
export function toggleOperation(
  permissions: Permissions,
  model: string,
  op: Operation,
  on: boolean,
): Permissions {
  if (op === "view" && !on) {
    const next = { ...permissions };
    delete next[model];
    return next;
  }
  const grant = { ...emptyGrant(), ...permissions[model], [op]: on };
  if (on) grant.view = true;
  return { ...permissions, [model]: grant };
}

export function toggleAction(
  permissions: Permissions,
  model: string,
  action: string,
  on: boolean,
): Permissions {
  const grant = { ...emptyGrant(), ...permissions[model] };
  const actions = grant.actions.filter((a) => a !== action);
  if (on) {
    actions.push(action);
    grant.view = true;
  }
  return { ...permissions, [model]: { ...grant, actions } };
}

/** Labels of the models a role can view, for the roles list. */
export function summary(permissions: Permissions, models: ModelMeta[]): string {
  const visible = models.filter((m) => grantsAnything(permissions[m.name])).map((m) => m.label);
  return visible.length > 0 ? visible.join(", ") : "Nothing";
}

/**
 * Foreign keys the role can set but whose target model it cannot view: the
 * admin shows those as plain id fields instead of searchable pickers.
 */
export function plainIdWarnings(permissions: Permissions, models: ModelMeta[]): string[] {
  const warnings: string[] = [];
  for (const model of models) {
    const grant = permissions[model.name];
    if (!grant) continue;
    const writes =
      (grant.create && model.capabilities.create) || (grant.update && model.capabilities.update);
    if (!writes) continue;
    for (const field of model.fields) {
      const target = field.relation?.model;
      if (!target || field.readonly || grantsAnything(permissions[target])) continue;
      const label = models.find((m) => m.name === target)?.label ?? target;
      warnings.push(
        `${model.label} › ${field.label} will be a plain id field, because this role cannot view ${label}.`,
      );
    }
  }
  return warnings;
}
