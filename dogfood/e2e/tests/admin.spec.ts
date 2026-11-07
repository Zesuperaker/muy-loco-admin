import { readFile } from "node:fs/promises";
import { type Page, expect, test } from "@playwright/test";

const PASSWORD = "12341234";

async function signIn(page: Page, email = "user1@example.com") {
  await page.goto("/admin");
  await page.getByLabel("Email").fill(email);
  await page.getByLabel("Password").fill(PASSWORD);
  await page.getByRole("button", { name: "Sign in" }).click();
}

async function signedIn(page: Page, path: string) {
  await signIn(page);
  await expect(page.getByRole("heading", { name: "Dashboard" })).toBeVisible();
  await page.goto(path);
}

test("wraps admin responses in the app's middleware", async ({ request }) => {
  const res = await request.get("/admin", { headers: { "accept-encoding": "gzip" } });
  const headers = res.headers();
  expect(headers["content-encoding"]).toBe("gzip");
  expect(headers["x-request-id"]).toBeTruthy();
  expect(headers["strict-transport-security"]).toBeTruthy();
  expect(headers["content-security-policy"]).toContain("script-src 'self'");
});

test("refuses users without admin access", async ({ page }) => {
  await signIn(page, "user2@example.com");
  await expect(page.getByText("Your account does not have admin access.")).toBeVisible();
  await page.getByRole("link", { name: "Visit site" }).click();
  await expect(page).toHaveURL(/\/$/);
  await expect(page.getByRole("link", { name: "Open the admin" })).toBeVisible();
});

test("rejects wrong passwords", async ({ page }) => {
  await page.goto("/admin");
  await page.getByLabel("Email").fill("user1@example.com");
  await page.getByLabel("Password").fill("wrong-password");
  await page.getByRole("button", { name: "Sign in" }).click();
  await expect(page.getByRole("alert")).toHaveText("Invalid credentials");
});

test("links back to the site", async ({ page }) => {
  await signIn(page);
  await page.getByRole("link", { name: "Visit site" }).click();
  await expect(page).toHaveURL(/\/$/);
  await expect(page.getByRole("link", { name: "Open the admin" })).toBeVisible();
});

test("shows record counts on the dashboard", async ({ page }) => {
  await signIn(page);
  await expect(page.getByTestId("count-articles")).toHaveText("12");
  await expect(page.getByTestId("count-users")).toHaveText("2");
  await expect(page.getByTestId("count-article_tags")).toHaveText("8");
});

test("searches, filters and sorts a list", async ({ page }) => {
  await signedIn(page, "/admin/models/articles");
  await expect(page.getByText("12 total")).toBeVisible();

  await page.getByLabel("Search").fill("uptime");
  await page.getByRole("button", { name: "Search" }).click();
  await expect(page.getByText("1 total")).toBeVisible();
  await expect(page.locator("tbody tr")).toContainText(["100% uptime is a myth"]);

  await page.getByRole("button", { name: "Clear", exact: true }).click();
  await page.getByLabel("Status").selectOption("published");
  await expect(page.getByText("5 total")).toBeVisible();
  await page.getByRole("button", { name: /^Views/ }).click();
  await page.getByRole("button", { name: /^Views/ }).click();
  await expect(page).toHaveURL(/sort=views&order=desc/);
  await expect(page.locator("tbody tr").first()).toContainText("100% uptime is a myth");
});

test("opens a record from its row, linking its name", async ({ page }) => {
  await signedIn(page, "/admin/models/categories");
  const header = page.locator("thead th");
  await expect(header.nth(1)).toHaveText(/^Id/);
  await expect(header.last()).toHaveText("Updated at");
  const row = page.locator("tbody tr", { hasText: "product" });
  await expect(row.getByRole("link")).toHaveText("Product");
  await row.getByRole("cell", { name: "product", exact: true }).click();
  await expect(page).toHaveURL(/\/admin\/models\/categories\/records\/2$/);
  await expect(page.getByRole("heading", { name: "Product" })).toBeVisible();
});

test("follows foreign keys and related lists", async ({ page }) => {
  await signedIn(page, "/admin/models/articles/records/3");
  await expect(page.getByLabel("User")).toHaveValue("user2@example.com");
  await page.goto("/admin/models/users/records/2");
  await expect(page.getByRole("heading", { name: "user2@example.com" })).toBeVisible();
  await page.getByRole("link", { name: "Articles" }).last().click();
  await expect(page).toHaveURL(/filter\.user_id=2/);
  await expect(page.getByText("6 total")).toBeVisible();
});

test("creates, edits and deletes a record", async ({ page }) => {
  const slug = `e2e-${Date.now()}`;
  await signedIn(page, "/admin/models/categories");
  await page.getByRole("link", { name: "Add record" }).click();
  await page.getByLabel("Name").fill("E2E category");
  await page.getByLabel("Slug").fill(slug);
  await page.getByRole("button", { name: "Create" }).click();
  await expect(page.getByText("Created E2E category")).toBeVisible();
  await expect(page.getByRole("heading", { name: "E2E category" })).toBeVisible();

  await page.getByLabel("Description").fill("Made by Playwright");
  await page.getByRole("button", { name: "Save" }).click();
  await expect(page.getByText("Saved", { exact: true })).toBeVisible();

  page.once("dialog", (dialog) => dialog.accept());
  await page.getByRole("button", { name: "Delete" }).click();
  await expect(page).toHaveURL(/\/admin\/models\/categories$/);
  await expect(page.getByText(slug)).toHaveCount(0);
});

test("reports duplicate values and hook validation errors", async ({ page }) => {
  await signedIn(page, "/admin/models/categories/new");
  await page.getByLabel("Name").fill("Duplicate");
  await page.getByLabel("Slug").fill("engineering");
  await page.getByRole("button", { name: "Create" }).click();
  await expect(page.getByRole("alert")).toContainText("a record with the same value already exists");

  await page.goto("/admin/models/articles/records/3");
  await page.getByLabel("Rating").fill("9");
  await page.getByRole("button", { name: "Save" }).click();
  await expect(page.getByText("rating must be between 0 and 5")).toBeVisible();
});

test("picks a foreign key", async ({ page }) => {
  await signedIn(page, "/admin/models/articles/records/9");
  const category = page.getByLabel("Category", { exact: true });
  await category.click();
  await category.fill("Prod");
  await page.getByRole("option", { name: /Product/ }).click();
  await page.getByRole("button", { name: "Save" }).click();
  await expect(page.getByText("Saved", { exact: true })).toBeVisible();
  await page.reload();
  await expect(page.getByLabel("Category", { exact: true })).toHaveValue("Product");
});

test("runs a bulk action", async ({ page }) => {
  await signedIn(page, "/admin/models/articles?filter.status=draft");
  await expect(page.getByText("4 total")).toBeVisible();
  await page.getByLabel("Select Shipping weekly releases").check();
  await page.getByLabel("Select Remote team rituals").check();
  await page.getByRole("button", { name: "Publish", exact: true }).click();
  await expect(page.getByText("Published 2 articles")).toBeVisible();
  await expect(page.getByText("2 total")).toBeVisible();
});

test("exports CSV", async ({ page }) => {
  await signedIn(page, "/admin/models/categories");
  const download = page.waitForEvent("download");
  await page.getByRole("button", { name: "Export CSV" }).click();
  const file = await download;
  expect(file.suggestedFilename()).toBe("categories.csv");
  const text = await readFile(await file.path(), "utf8");
  expect(text.split("\n")[0]).toBe("id,name,slug,description,created_at,updated_at");
  expect(text).toContain("engineering");
});

test("edits a composite-key record", async ({ page }) => {
  await signedIn(page, "/admin/models/article_tags/records/1%2C2");
  await expect(page.getByLabel("Article")).toHaveText("Getting started with Loco");
  await page.getByLabel("Position").fill("7");
  await page.getByRole("button", { name: "Save" }).click();
  await expect(page.getByText("Saved", { exact: true })).toBeVisible();
});

test("keeps users read-only for creation and signs out", async ({ page }) => {
  await signedIn(page, "/admin/models/users");
  await expect(page.getByText("2 total")).toBeVisible();
  await expect(page.getByRole("link", { name: "Add record" })).toHaveCount(0);
  await page.getByRole("button", { name: "Sign out" }).click();
  await expect(page.getByText("Sign in to continue.")).toBeVisible();
});

test("keeps the session in an HttpOnly cookie that signing out ends", async ({ page, request }) => {
  await signedIn(page, "/admin");
  const session = (await page.context().cookies()).find((c) => c.name === "__Host-loco_session");
  expect(session).toMatchObject({ httpOnly: true, secure: true, sameSite: "Lax", path: "/" });
  expect(await page.evaluate(() => localStorage.length)).toBe(0);
  expect(await page.evaluate(() => document.cookie)).not.toContain("loco_session");

  const copied = { cookie: `${session!.name}=${session!.value}` };
  expect((await request.get("/admin/api/me", { headers: copied })).status()).toBe(200);
  await page.getByRole("button", { name: "Sign out" }).click();
  await expect(page.getByText("Sign in to continue.")).toBeVisible();
  expect((await request.get("/admin/api/me", { headers: copied })).status()).toBe(401);
});
