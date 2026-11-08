import { useQuery } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { api } from "../api";

/** A select of the roles, for the users' role column. */
export function RoleSelect({
  id,
  value,
  label,
  disabled = false,
  emptyLabel,
  children,
  onChange,
}: {
  id: string;
  value: string;
  /** The name of the role `value` names, if known. */
  label?: string;
  disabled?: boolean;
  /** The option for no role. */
  emptyLabel: string;
  /** More options after the roles. */
  children?: ReactNode;
  onChange: (value: string) => void;
}) {
  const roles = useQuery({ queryKey: ["roles"], queryFn: api.roles });
  // Until the roles load, or when they cannot, keep showing the role the
  // value names rather than the option for no role.
  const unlisted =
    /^\d+$/.test(value) && !(roles.data?.roles.some((role) => String(role.id) === value) ?? false);
  return (
    <>
      <select
        id={id}
        className="input"
        value={value}
        disabled={disabled || roles.isPending}
        onChange={(e) => onChange(e.target.value)}
      >
        <option value="">{emptyLabel}</option>
        {unlisted && <option value={value}>{label ?? `Role ${value}`}</option>}
        {roles.data?.roles.map((role) => (
          <option key={role.id} value={String(role.id)}>
            {role.name}
          </option>
        ))}
        {children}
      </select>
      {roles.isError && <p className="text-xs text-rose-600">The roles could not be loaded.</p>}
    </>
  );
}
