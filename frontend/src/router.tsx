import { type AnchorHTMLAttributes, type MouseEvent, useSyncExternalStore } from "react";
import { basePath } from "./api";

export type Route =
  | { name: "dashboard" }
  | { name: "list"; model: string }
  | { name: "new"; model: string }
  | { name: "record"; model: string; key: string }
  | { name: "not-found" };

/** Match a path relative to the mount path. */
export function parseRoute(path: string): Route {
  const parts = path.split("/").filter(Boolean).map(decodeURIComponent);
  if (parts.length === 0) return { name: "dashboard" };
  if (parts[0] !== "models" || !parts[1]) return { name: "not-found" };
  const model = parts[1];
  if (parts.length === 2) return { name: "list", model };
  if (parts.length === 3 && parts[2] === "new") return { name: "new", model };
  if (parts.length === 4 && parts[2] === "records" && parts[3] !== undefined) {
    return { name: "record", model, key: parts[3] };
  }
  return { name: "not-found" };
}

export const paths = {
  dashboard: () => "/",
  list: (model: string, search = "") => `/models/${encodeURIComponent(model)}${search}`,
  create: (model: string) => `/models/${encodeURIComponent(model)}/new`,
  record: (model: string, key: string) =>
    `/models/${encodeURIComponent(model)}/records/${encodeURIComponent(key)}`,
};

const listeners = new Set<() => void>();

function subscribe(listener: () => void) {
  listeners.add(listener);
  window.addEventListener("popstate", listener);
  return () => {
    listeners.delete(listener);
    window.removeEventListener("popstate", listener);
  };
}

function snapshot(): string {
  return window.location.pathname + window.location.search;
}

/** The current path relative to the mount path, and its query string. */
export function useLocation(): { path: string; search: URLSearchParams } {
  const full = useSyncExternalStore(subscribe, snapshot, snapshot);
  const [pathname = "", query = ""] = full.split("?");
  const base = basePath();
  const path = pathname.startsWith(base) ? pathname.slice(base.length) || "/" : pathname;
  return { path, search: new URLSearchParams(query) };
}

export function href(to: string): string {
  return `${basePath()}${to === "/" ? "" : to}` || "/";
}

export function navigate(to: string, { replace = false } = {}): void {
  const url = href(to);
  if (replace) {
    window.history.replaceState(null, "", url);
  } else {
    window.history.pushState(null, "", url);
  }
  listeners.forEach((listener) => listener());
  window.scrollTo?.(0, 0);
}

type LinkProps = AnchorHTMLAttributes<HTMLAnchorElement> & { to: string };

export function Link({ to, onClick, children, ...rest }: LinkProps) {
  const handle = (event: MouseEvent<HTMLAnchorElement>) => {
    onClick?.(event);
    if (
      event.defaultPrevented ||
      event.button !== 0 ||
      event.metaKey ||
      event.ctrlKey ||
      event.shiftKey ||
      event.altKey
    ) {
      return;
    }
    event.preventDefault();
    navigate(to);
  };
  return (
    <a href={href(to)} onClick={handle} {...rest}>
      {children}
    </a>
  );
}
