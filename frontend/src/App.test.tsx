import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { getToken, setToken } from "./api";
import { baseRoutes, currentLocation, formConfig, mockFetch, renderApp } from "./test/utils";

describe("App", () => {
  it("signs in through the configured login form", async () => {
    let signedIn = false;
    const { calls } = mockFetch(
      baseRoutes({
        "GET /admin/api/me": () =>
          signedIn
            ? { body: { id: "1", name: "Ann" } }
            : { status: 401, body: { error: "authentication required" } },
        "POST /api/auth/login": (call) => {
          const body = call.body as { email: string; password: string };
          if (body.password !== "secret") return { status: 401, body: {} };
          signedIn = true;
          return { body: { token: "jwt-token" } };
        },
      }),
    );
    const user = userEvent.setup();
    renderApp();

    expect(await screen.findByText("Sign in to continue.")).toBeInTheDocument();
    await user.type(screen.getByLabelText("Email"), "ann@example.com");
    await user.type(screen.getByLabelText("Password"), "wrong");
    await user.click(screen.getByRole("button", { name: "Sign in" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Invalid credentials");

    await user.clear(screen.getByLabelText("Password"));
    await user.type(screen.getByLabelText("Password"), "secret");
    await user.click(screen.getByRole("button", { name: "Sign in" }));

    expect(await screen.findByRole("heading", { name: "Dashboard" })).toBeInTheDocument();
    expect(getToken()).toBe("jwt-token");
    expect(await screen.findByTestId("count-posts")).toHaveTextContent("5");
    const schemaCall = calls.find((c) => c.url === "/admin/api/schema");
    expect(schemaCall?.headers.get("Authorization")).toBe("Bearer jwt-token");
    expect(document.title).toBe("Test Admin");

    await user.click(screen.getByRole("button", { name: "Sign out" }));
    expect(getToken()).toBeNull();
  });

  it("explains when the account lacks admin access", async () => {
    mockFetch(baseRoutes({ "GET /admin/api/me": { status: 403, body: { error: "forbidden" } } }));
    renderApp();
    expect(await screen.findByText("Your account does not have admin access.")).toBeInTheDocument();
  });

  it("offers a redirect or retry for other login methods", async () => {
    mockFetch(
      baseRoutes({
        "GET /admin/api/config": { body: { ...formConfig, login: { type: "redirect", url: "/login" } } },
        "GET /admin/api/me": { status: 401, body: {} },
      }),
    );
    const { unmount } = renderApp();
    expect(await screen.findByRole("link", { name: "Continue to sign in" })).toHaveAttribute(
      "href",
      "/login",
    );
    unmount();

    mockFetch(
      baseRoutes({
        "GET /admin/api/config": { body: { ...formConfig, login: { type: "none" } } },
        "GET /admin/api/me": { status: 401, body: {} },
      }),
    );
    renderApp();
    expect(await screen.findByRole("button", { name: "Retry" })).toBeInTheDocument();
  });

  it("shows configuration and server errors", async () => {
    mockFetch({ "GET /admin/api/config": { status: 500, body: { error: "config broke" } } });
    const { unmount } = renderApp();
    expect(await screen.findByRole("alert")).toHaveTextContent("config broke");
    unmount();

    mockFetch(baseRoutes({ "GET /admin/api/me": { status: 500, body: { error: "me broke" } } }));
    const second = renderApp();
    expect(await screen.findByRole("alert")).toHaveTextContent("me broke");
    second.unmount();

    mockFetch(baseRoutes({ "GET /admin/api/schema": { status: 500, body: { error: "schema broke" } } }));
    renderApp();
    expect(await screen.findByRole("alert")).toHaveTextContent("schema broke");
  });

  it("drops a rejected token and returns to the login screen", async () => {
    setToken("stale");
    mockFetch(baseRoutes({ "GET /admin/api/me": { status: 401, body: {} } }));
    renderApp();
    expect(await screen.findByText("Sign in to continue.")).toBeInTheDocument();
    await waitFor(() => expect(getToken()).toBeNull());
  });

  it("renders navigation and unknown routes", async () => {
    mockFetch(baseRoutes());
    const user = userEvent.setup();
    renderApp("/admin/nowhere");
    expect(await screen.findByRole("heading", { name: "Not found" })).toBeInTheDocument();
    await user.click(screen.getByRole("link", { name: "Back to the dashboard" }));
    expect(await screen.findByRole("heading", { name: "Dashboard" })).toBeInTheDocument();
    expect(screen.getByText("Ann")).toBeInTheDocument();

    await user.click(screen.getByRole("link", { name: "Posts" }));
    expect(currentLocation()).toBe("/models/posts");
  });

  it("reports unknown models", async () => {
    mockFetch(baseRoutes());
    renderApp("/admin/models/ghosts");
    expect(await screen.findByRole("heading", { name: "Not found" })).toBeInTheDocument();
  });

  it("shows an empty dashboard", async () => {
    mockFetch(baseRoutes({ "GET /admin/api/schema": { body: { models: [] } } }));
    renderApp();
    expect(await screen.findByText("No models are available to you.")).toBeInTheDocument();
  });
});
