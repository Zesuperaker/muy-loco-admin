/** URL builders for the admin's routes, relative to the mount path. */
export const paths = {
  dashboard: () => "/",
  list: (model: string, search = "") => `/models/${encodeURIComponent(model)}${search}`,
  create: (model: string) => `/models/${encodeURIComponent(model)}/new`,
  record: (model: string, key: string) =>
    `/models/${encodeURIComponent(model)}/records/${encodeURIComponent(key)}`,
  roles: () => "/roles",
  newRole: () => "/roles/new",
  role: (id: number) => `/roles/${id}`,
};
