import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/preact";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { PluginInfo, Resources, StoreEntry } from "../../api";
import { mockApi, resetStores } from "../../test-utils";
import { resetRecipeUpdates } from "../layout/useRecipeUpdates";
import { Sidebar } from "../layout/Sidebar";
import { PluginsPage } from "./PluginsPage";

const PLUGINS = "GET /api/v1/plugins";
const STORE = "GET /api/v1/plugins/store";
const REFRESH = "POST /api/v1/plugins/store/refresh";
const INSTALL = "POST /api/v1/plugins/install";
const UPDATE = "POST /api/v1/plugins/a/update";
const UNINSTALL_A = "POST /api/v1/plugins/a/uninstall";
const UNINSTALL_BAD = "POST /api/v1/plugins/bad/uninstall";
const RESOURCES = "GET /api/v1/system/resources";
const REFRESHED = { body: { count: 1, source: "remote" } };

afterEach(() => {
  cleanup();
  resetStores();
  resetRecipeUpdates();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

/** A catalogue recipe not installed, Sowel's store entry. */
const entry = (id: string, version: string, extra: Partial<StoreEntry> = {}): StoreEntry => ({
  id,
  version,
  name: `Recipe ${id}`,
  description: `What ${id} does`,
  repo: `solite-recipes/${id}`,
  type: "recipe",
  compatible: true,
  isOfficial: true,
  tier: "official",
  ...extra,
});

/** An installed recipe, Sowel's `PluginInfo`. */
const plugin = (
  id: string,
  version: string,
  extra: { latestVersion?: string; name?: string } = {},
): PluginInfo => ({
  manifest: {
    id,
    name: extra.name ?? `Recipe ${id}`,
    version,
    description: `What ${id} does`,
    repo: `solite-recipes/${id}`,
    type: "recipe",
  },
  enabled: true,
  installedAt: "2026-11-27T10:00:00.000Z",
  status: "connected",
  deviceCount: 0,
  offlineDeviceCount: 0,
  latestVersion: extra.latestVersion,
  source: "registry",
});

const resources = (
  refused: string[] = [],
  catalogue: Resources["catalogue"] = { readAt: "2026-11-27T10:00:00.000Z", error: null },
): Resources => ({
  internal: null,
  psram: null,
  storage: { used: 0, total: null },
  scripts: refused.map((id) => ({
    id,
    version: null,
    source: "installed",
    state: "refused",
    reason: "Invalid signature",
    held: 0,
    limit: 0,
    instances: 0,
    calls: 0,
    averageUs: 0,
    maxUs: 0,
  })),
  catalogue,
});

/** Remove asks a second click ("Confirm?") before it uninstalls. */
const remove = (row: HTMLElement) => {
  fireEvent.click(within(row).getByRole("button", { name: "Remove" }));
  fireEvent.click(within(row).getByRole("button", { name: "Confirm?" }));
};

const tab = (name: RegExp) => fireEvent.click(screen.getByRole("tab", { name }));

describe("PluginsPage", () => {
  it("lists the installed recipes first, with the counts of both tabs", async () => {
    mockApi({
      [PLUGINS]: { body: [plugin("a", "1.0.0")] },
      [STORE]: { body: [entry("b", "2.0.0"), entry("c", "1.0.0")] },
      [REFRESH]: REFRESHED,
      [RESOURCES]: { body: resources() },
    });
    render(<PluginsPage />);
    const a = await screen.findByTestId("recipe-a");
    expect(a.textContent).toContain("Recipe a");
    expect(a.textContent).toContain("1.0.0");
    expect(within(a).getByRole("button", { name: "Remove" })).toBeTruthy();
    expect(within(a).queryByRole("button", { name: "Update" })).toBeNull();
    expect(screen.queryByTestId("recipe-b")).toBeNull();
    expect(screen.getByRole("tab", { name: /^Installed/ }).textContent).toBe("Installed1");
    expect(screen.getByRole("tab", { name: /^Store/ }).textContent).toBe("Store2");
  });

  it("lists the catalogue in the Store tab, reading it again when the page opens", async () => {
    const api = mockApi({
      [PLUGINS]: { body: [] },
      [STORE]: [
        { body: [entry("a", "1.0.0")] },
        { body: [entry("a", "1.0.0"), entry("b", "2.0.0")] },
      ],
      [REFRESH]: REFRESHED,
      [RESOURCES]: { body: resources() },
    });
    render(<PluginsPage />);
    await screen.findByText("No plugins installed");
    tab(/^Store/);
    expect(await screen.findByTestId("recipe-b")).toBeTruthy();
    expect(api.count(REFRESH)).toBe(1);
    const a = screen.getByTestId("recipe-a");
    expect(a.textContent).toContain("What a does");
    expect(within(a).getByRole("button", { name: "Install" })).toBeTruthy();
  });

  it("shows the list held when the refresh answers 429", async () => {
    mockApi({
      [PLUGINS]: { body: [] },
      [STORE]: { body: [entry("a", "1.0.0")] },
      [REFRESH]: { status: 429, body: { error: "Please wait 42s before refreshing again" } },
      [RESOURCES]: { body: resources() },
    });
    render(<PluginsPage />);
    await screen.findByText("No plugins installed");
    tab(/^Store/);
    expect(await screen.findByTestId("recipe-a")).toBeTruthy();
    expect(screen.queryByText(/Please wait/)).toBeNull();
  });

  it("installs a recipe by its repo from the Store tab and shows it installed", async () => {
    const api = mockApi({
      [PLUGINS]: [{ body: [] }, { body: [] }, { body: [plugin("a", "1.0.0")] }],
      [STORE]: [{ body: [entry("a", "1.0.0", { repo: "someone/sowel-recipe-a" })] }, { body: [] }],
      [REFRESH]: REFRESHED,
      [INSTALL]: { body: { success: true } },
      [RESOURCES]: { body: resources() },
    });
    render(<PluginsPage />);
    await screen.findByText("No plugins installed");
    tab(/^Store/);
    fireEvent.click(
      within(await screen.findByTestId("recipe-a")).getByRole("button", { name: "Install" }),
    );
    await screen.findByText("Recipe installed.");
    expect(api.calls.find((c) => c.key === INSTALL)?.body).toEqual({
      repo: "someone/sowel-recipe-a",
    });
    await waitFor(() => expect(screen.queryByTestId("recipe-a")).toBeNull());
    tab(/^Installed/);
    expect(
      within(await screen.findByTestId("recipe-a")).getByRole("button", { name: "Remove" }),
    ).toBeTruthy();
  });

  it("offers Update when the catalogue has a newer version, and Remove", async () => {
    const api = mockApi({
      [PLUGINS]: { body: [plugin("a", "1.0.0", { latestVersion: "1.0.1" })] },
      [STORE]: { body: [] },
      [REFRESH]: REFRESHED,
      [UPDATE]: { body: { success: true } },
      [UNINSTALL_A]: { body: { success: true } },
      [RESOURCES]: { body: resources() },
    });
    render(<PluginsPage />);
    const a = await screen.findByTestId("recipe-a");
    expect(a.textContent).toContain("1.0.1");
    fireEvent.click(within(a).getByRole("button", { name: "Update" }));
    await screen.findByText("Recipe updated.");
    expect(api.count(UPDATE)).toBe(1);
    remove(screen.getByTestId("recipe-a"));
    await screen.findByText("Recipe removed.");
    expect(api.count(UNINSTALL_A)).toBe(1);
  });

  it("shows the server's reason when an install is refused", async () => {
    mockApi({
      [PLUGINS]: { body: [] },
      [STORE]: { body: [entry("a", "1.0.0")] },
      [REFRESH]: REFRESHED,
      [INSTALL]: {
        status: 409,
        body: { error: "Not enough memory for a: 700 KB free, 736 KB needed" },
      },
      [RESOURCES]: { body: resources() },
    });
    render(<PluginsPage />);
    await screen.findByText("No plugins installed");
    tab(/^Store/);
    fireEvent.click(
      within(await screen.findByTestId("recipe-a")).getByRole("button", { name: "Install" }),
    );
    expect(
      await screen.findByText("Not enough memory for a: 700 KB free, 736 KB needed"),
    ).toBeTruthy();
  });

  it("offers no upload of a signed recipe", async () => {
    mockApi({
      [PLUGINS]: { body: [] },
      [STORE]: { body: [] },
      [REFRESH]: REFRESHED,
      [RESOURCES]: { body: resources() },
    });
    render(<PluginsPage />);
    await screen.findByText("No plugins installed");
    expect(screen.queryByRole("button", { name: /upload/i })).toBeNull();
    expect(screen.queryByLabelText(/signed recipe/i)).toBeNull();
  });

  it("lists a recipe the catalogue lacks as installed, and removes it", async () => {
    const api = mockApi({
      [PLUGINS]: [{ body: [plugin("mine", "0.1.0")] }, { body: [] }],
      [STORE]: { body: [] },
      [REFRESH]: REFRESHED,
      [RESOURCES]: { body: resources() },
      "POST /api/v1/plugins/mine/uninstall": { body: { success: true } },
    });
    render(<PluginsPage />);
    const mine = await screen.findByTestId("recipe-mine");
    expect(mine.textContent).toContain("0.1.0");
    remove(mine);
    await screen.findByText("Recipe removed.");
    expect(api.count("POST /api/v1/plugins/mine/uninstall")).toBe(1);
  });

  it("lists a refused file with its reason, read from the resources, and removes it", async () => {
    const api = mockApi({
      [PLUGINS]: { body: [] },
      [STORE]: { body: [] },
      [REFRESH]: REFRESHED,
      [RESOURCES]: [
        { body: resources(["bad"]) },
        { body: resources(["bad"]) },
        { body: resources() },
      ],
      [UNINSTALL_BAD]: { body: { success: true } },
    });
    render(<PluginsPage />);
    const bad = await screen.findByTestId("refused-bad");
    expect(bad.textContent).toContain("File refused: Invalid signature");
    remove(bad);
    await screen.findByText("Recipe removed.");
    expect(api.count(UNINSTALL_BAD)).toBe(1);
    await waitFor(() => expect(screen.queryByTestId("refused-bad")).toBeNull());
  });

  it("shows the error of the last read, from the resources, in the Store tab and keeps the list", async () => {
    mockApi({
      [PLUGINS]: { body: [] },
      [STORE]: { body: [entry("a", "1.0.0")] },
      [REFRESH]: { body: { count: 1, source: "local" } },
      [RESOURCES]: {
        body: resources([], { readAt: "2026-11-27T10:00:00.000Z", error: "no route to host" }),
      },
    });
    render(<PluginsPage />);
    await screen.findByText("No plugins installed");
    tab(/^Store/);
    expect((await screen.findByTestId("read-error")).textContent).toBe(
      "The catalogue could not be read: no route to host",
    );
    expect(screen.getByTestId("recipe-a")).toBeTruthy();
  });

  it("searches the list of the active tab by id, name and description", async () => {
    mockApi({
      [PLUGINS]: { body: [plugin("a", "1.0.0"), plugin("b", "1.0.0", { name: "Night light" })] },
      [STORE]: { body: [entry("a2", "1.0.0"), entry("b2", "1.0.0")] },
      [REFRESH]: REFRESHED,
      [RESOURCES]: { body: resources() },
    });
    render(<PluginsPage />);
    await screen.findByTestId("recipe-a");
    const field = screen.getByLabelText("Search plugins...");
    fireEvent.input(field, { target: { value: "night" } });
    expect(screen.queryByTestId("recipe-a")).toBeNull();
    expect(screen.getByTestId("recipe-b")).toBeTruthy();
    fireEvent.input(field, { target: { value: "zzz" } });
    expect(screen.getByText('No plugins match "zzz"')).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Clear" }));
    expect(screen.getByTestId("recipe-a")).toBeTruthy();
    tab(/^Store/);
    fireEvent.input(field, { target: { value: "what b2" } });
    expect(screen.queryByTestId("recipe-a2")).toBeNull();
    expect(screen.getByTestId("recipe-b2")).toBeTruthy();
  });

  it("uninstalls only on the second click of Remove", async () => {
    const api = mockApi({
      [PLUGINS]: { body: [plugin("a", "1.0.0")] },
      [STORE]: { body: [] },
      [REFRESH]: REFRESHED,
      [RESOURCES]: { body: resources() },
      [UNINSTALL_A]: { body: { success: true } },
    });
    render(<PluginsPage />);
    const a = await screen.findByTestId("recipe-a");
    fireEvent.click(within(a).getByRole("button", { name: "Remove" }));
    expect(within(a).getByRole("button", { name: "Confirm?" })).toBeTruthy();
    expect(api.count(UNINSTALL_A)).toBe(0);
    fireEvent.focusOut(within(a).getByRole("button", { name: "Confirm?" }));
    expect(await within(a).findByRole("button", { name: "Remove" })).toBeTruthy();
    expect(api.count(UNINSTALL_A)).toBe(0);
  });

  it("offers no Remove for a recipe of the host's own script, and no Install for an incompatible entry", async () => {
    const own: Resources = {
      ...resources(),
      scripts: [
        {
          id: "a",
          version: "1.0.0",
          source: "builtIn",
          state: "ready",
          reason: null,
          held: 0,
          limit: 0,
          instances: 0,
          calls: 0,
          averageUs: 0,
          maxUs: 0,
        },
      ],
    };
    mockApi({
      [PLUGINS]: { body: [plugin("a", "1.0.0"), plugin("b", "1.0.0")] },
      [STORE]: {
        body: [
          entry("c", "1.0.0", { compatible: false, compatReason: "Requires Solite >= 9.0.0" }),
          entry("d", "1.0.0"),
        ],
      },
      [REFRESH]: REFRESHED,
      [RESOURCES]: { body: own },
    });
    render(<PluginsPage />);
    const a = await screen.findByTestId("recipe-a");
    const b = screen.getByTestId("recipe-b");
    await waitFor(() => expect(within(a).queryByRole("button", { name: "Remove" })).toBeNull());
    expect(within(b).getByRole("button", { name: "Remove" })).toBeTruthy();
    tab(/^Store/);
    const c = await screen.findByTestId("recipe-c");
    expect(
      within(c)
        .getByRole("button", { name: /Install/ })
        .hasAttribute("disabled"),
    ).toBe(true);
    expect(within(c).getByText("Requires Solite >= 9.0.0")).toBeTruthy();
    expect(
      within(screen.getByTestId("recipe-d"))
        .getByRole("button", { name: /Install/ })
        .hasAttribute("disabled"),
    ).toBe(false);
  });

  it("says why when the refresh at opening is refused, not that the catalogue is empty", async () => {
    mockApi({
      [PLUGINS]: { body: [] },
      [STORE]: { body: [] },
      [REFRESH]: { status: 503, body: { error: "Not enough memory for the catalogue" } },
      [RESOURCES]: { body: resources([], { readAt: null, error: null }) },
    });
    render(<PluginsPage />);
    expect((await screen.findByRole("alert")).textContent).toContain("Not enough memory");
    tab(/^Store/);
    expect(screen.queryByText("The catalogue lists no recipe.")).toBeNull();
  });

  it("keeps the list silently when the refresh is paced (429)", async () => {
    mockApi({
      [PLUGINS]: { body: [plugin("a", "1.0.0")] },
      [STORE]: { body: [] },
      [REFRESH]: { status: 429, body: { error: "Too soon" } },
      [RESOURCES]: { body: resources() },
    });
    render(<PluginsPage />);
    await screen.findByTestId("recipe-a");
    await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
  });

  it("clears the error of the first read when the refresh succeeds", async () => {
    mockApi({
      [PLUGINS]: [
        { status: 500, body: { error: "Held list unreadable" } },
        { body: [plugin("a", "1.0.0")] },
      ],
      [STORE]: { body: [] },
      [REFRESH]: REFRESHED,
      [RESOURCES]: { body: resources() },
    });
    render(<PluginsPage />);
    await screen.findByTestId("recipe-a");
    await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
  });

  it("feeds the sidebar's badge: an Update takes the count down", async () => {
    const before = plugin("a", "1.0.0", { latestVersion: "2.0.0" });
    mockApi({
      [PLUGINS]: [
        { body: [before] },
        { body: [before] },
        { body: [before] },
        { body: [plugin("a", "2.0.0")] },
      ],
      [STORE]: { body: [] },
      [REFRESH]: REFRESHED,
      [RESOURCES]: { body: resources() },
      [UPDATE]: { body: { success: true } },
    });
    render(
      <>
        <Sidebar zones={[]} activeId={null} page="plugins" />
        <PluginsPage />
      </>,
    );
    expect((await screen.findByTestId("plugins-badge")).textContent).toBe("1");
    fireEvent.click(await screen.findByRole("button", { name: "Update" }));
    await waitFor(() => expect(screen.queryByTestId("plugins-badge")).toBeNull());
  });
});
