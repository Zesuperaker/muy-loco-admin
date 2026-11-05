import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { BrowserRouter } from "react-router";
import { App } from "./App";
import { ApiError, basePath } from "./api";
import { ToastProvider } from "./components/ui";
import "./index.css";

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      refetchOnWindowFocus: false,
      retry: (count, error) =>
        !(error instanceof ApiError && error.status < 500) && count < 2,
    },
  },
});

const root = document.getElementById("root");
if (root) {
  createRoot(root).render(
    <StrictMode>
      <QueryClientProvider client={queryClient}>
        <BrowserRouter basename={basePath()}>
          <ToastProvider>
            <App />
          </ToastProvider>
        </BrowserRouter>
      </QueryClientProvider>
    </StrictMode>,
  );
}
