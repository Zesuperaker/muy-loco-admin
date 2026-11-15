import { type QueryClientConfig, replaceEqualDeep } from "@tanstack/react-query";
import { ApiError } from "./api";
import { hasRawJson } from "./json";

/**
 * React Query's structural sharing, which keeps unchanged parts of refetched
 * data identical, except for data holding `JSON.rawJSON` values: it would
 * copy those into plain `{ rawJSON }` objects, which no longer write
 * themselves back as the number they stand for.
 */
export function shareData(previous: unknown, next: unknown): unknown {
  return hasRawJson(next) ? next : replaceEqualDeep(previous, next);
}

export const queryClientConfig: QueryClientConfig = {
  defaultOptions: {
    queries: {
      refetchOnWindowFocus: false,
      retry: (count, error) => !(error instanceof ApiError && error.status < 500) && count < 2,
      structuralSharing: shareData,
    },
  },
};
