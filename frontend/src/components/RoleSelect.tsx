import { useQuery } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { api } from "../api";

/** Whether the signed-in user may manage and assign roles. */
export function useCanManageRoles(): boolean {
  const me = useQuery({ queryKey: ["me"], queryFn: api.me, staleTime: Infinity });
  return me.data?.can_manage_roles === true;
}

/** A select of the roles, for the users' role column. */
export function RoleSelect({
  id,
  value,
  disabled = false,
  emptyLabel,
  children,
  onChange,
}: {
  id: string;
  value: string;
  disabled?: boolean;
  /** The option for no role. */
  emptyLabel: string;
  /** More options after the roles. */
  children?: ReactNode;
  onChange: (value: string) => void;
}) {
  const roles = useQuery({ queryKey: ["roles"], queryFn: api.roles });
  return (
    <select
      id={id}
      className="input"
      value={value}
      disabled={disabled || roles.isPending}
      onChange={(e) => onChange(e.target.value)}
    >
      <option value="">{emptyLabel}</option>
      {roles.data?.roles.map((role) => (
        <option key={role.id} value={String(role.id)}>
          {role.name}
        </option>
      ))}
      {children}
    </select>
  );
}
