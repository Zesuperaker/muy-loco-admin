import { createContext, useContext } from "react";
import type { CurrentUser } from "./types";

/** The signed-in user, provided by the shell around every page. */
export const UserContext = createContext<CurrentUser | null>(null);

/** Whether the signed-in user may manage and assign roles. */
export function useCanManageRoles(): boolean {
  return useContext(UserContext)?.can_manage_roles === true;
}
