import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/preact";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flattenZones } from "../../zones";
import { mockApi, resetStores, storeSession, zone } from "../../test-utils";
import { useAuth } from "../../store/useAuth";
import { resetRecipeUpdates } from "./useRecipeUpdates";
import { MobileDrawer, Sidebar, type Page } from "./Sidebar";

const flat = flattenZones([
  zone("root", "Home", [zone("garage", "Garage", [], "root"), zone("house", "House", [], "root")]),
]);

beforeEach(() => {
  window.location.hash = "";
});
afterEach(() => {
  cleanup();
  resetStores();
  resetRecipeUpdates();
  vi.unstubAllGlobals();
});

const show = (page: Page | null = null) =>
  render(<Sidebar zones={flat} activeId="garage" page={page} />);
const entries = () =>
  within(screen.getByRole("navigation"))
    .getAllByRole("link")
    .map((l) => l.textContent);
const title = () => screen.getByRole("button", { name: "Administration" });

describe("Sidebar, Administration section", () => {
  it("is closed on a room page, with the room marked and no entry", () => {
    show();
    expect(title().getAttribute("aria-expanded")).toBe("false");
    expect(entries()).toEqual(["Home", "Garage", "House"]);
    const marked = screen.getAllByRole("link").filter((a) => a.getAttribute("aria-current"));
    expect(marked.map((a) => a.textContent)).toEqual(["Garage"]);
  });

  it("opens and closes on a click of its title, entries in order", () => {
    show();
    fireEvent.click(title());
    expect(entries().slice(-6)).toEqual([
      "Devices",
      "Equipments",
      "Topology",
      "Discovery",
      "Plugins",
      "Settings",
    ]);
    expect(title().getAttribute("aria-expanded")).toBe("true");
    fireEvent.click(title());
    expect(entries()).toEqual(["Home", "Garage", "House"]);
  });

  it("sits in a bottom block under the scrolling room tree, as Sowel's (spec 047)", () => {
    show();
    const nav = screen.getByRole("navigation");
    const [tree, bottom, ...rest] = Array.from(nav.children);
    expect(rest).toEqual([]);
    expect(tree.className).toContain("overflow-y-auto");
    expect(tree.className).toContain("flex-1");
    expect(tree.className).toContain("min-h-0");
    expect(tree.textContent).toContain("Garage");
    for (const c of [
      "border-t",
      "border-border-light",
      "py-2",
      "px-2",
      "min-h-0",
      "overflow-y-auto",
    ])
      expect(bottom.className.split(" ")).toContain(c);
    const section = title().parentElement!;
    expect(section.parentElement).toBe(bottom);
    expect(section.className).not.toContain("border-t");
    // Log out follows, and nothing after it.
    expect(section.nextElementSibling?.textContent).toBe("Log out");
    expect(section.nextElementSibling?.nextElementSibling).toBeNull();
  });

  it("has the same bottom block in the phone's drawer", () => {
    render(<MobileDrawer zones={flat} activeId="garage" page={null} onClose={() => {}} />);
    const [tree, bottom] = Array.from(screen.getByRole("navigation").children);
    expect(tree.className).toContain("overflow-y-auto");
    expect(bottom.className).toContain("border-t");
    expect(bottom.textContent).toContain("Administration");
    expect(bottom.textContent).toContain("Log out");
  });

  it("keeps every room in the scrolling area, many or none", () => {
    const many = flattenZones([
      zone(
        "root",
        "Home",
        Array.from({ length: 20 }, (_, i) => zone(`z${i}`, `Room ${i}`, [], "root")),
      ),
    ]);
    render(<Sidebar zones={many} activeId="z0" page={null} />);
    const [tree, bottom] = Array.from(screen.getByRole("navigation").children);
    expect(tree.textContent).toContain("Room 19");
    expect(bottom.textContent).not.toContain("Room");
    cleanup();
    render(<Sidebar zones={[]} activeId={null} page={null} />);
    const nav = screen.getByRole("navigation");
    expect(nav.lastElementChild?.textContent).toContain("Administration");
    expect(nav.lastElementChild?.textContent).toContain("Log out");
  });

  it.each([
    ["devices", "Devices"],
    ["equipments", "Equipments"],
    ["zones", "Topology"],
    ["discovery", "Discovery"],
    ["plugins", "Plugins"],
    ["settings", "Settings"],
  ] as const)(
    "is open at %s with its entry marked, no room, and one mark in all",
    (page, label) => {
      show(page);
      expect(title().getAttribute("aria-expanded")).toBe("true");
      const marked = screen.getAllByRole("link").filter((a) => a.getAttribute("aria-current"));
      expect(marked.map((a) => a.textContent)).toEqual([label]);
    },
  );

  it("links each entry in the hash", () => {
    show("devices");
    const hrefs = Object.fromEntries(
      screen.getAllByRole("link").map((a) => [a.textContent, a.getAttribute("href")]),
    );
    expect(hrefs.Devices).toBe("#/devices");
    expect(hrefs.Equipments).toBe("#/equipments");
    expect(hrefs.Topology).toBe("#/zones");
    expect(hrefs.Discovery).toBe("#/discovery/solite");
    expect(hrefs.Settings).toBe("#/settings");
  });

  it("closes the drawer when an entry is picked, after setting the hash", () => {
    const onClose = vi.fn();
    render(<MobileDrawer zones={flat} activeId="garage" page={null} onClose={onClose} />);
    fireEvent.click(
      within(screen.getByRole("dialog")).getByRole("button", { name: "Administration" }),
    );
    fireEvent.click(within(screen.getByRole("dialog")).getByText("Devices"));
    expect(window.location.hash).toBe("#/devices");
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});

describe("Sidebar, Plugins entry (spec 042, R5.4)", () => {
  const catalogue = (installed: [string, string][], newer: string[]) =>
    installed.map(([id, version]) => ({
      manifest: {
        id,
        name: id,
        version,
        description: "",
        repo: `solite-recipes/${id}`,
        type: "recipe",
      },
      enabled: true,
      installedAt: null,
      status: "connected",
      deviceCount: 0,
      offlineDeviceCount: 0,
      ...(newer.includes(id) ? { latestVersion: "2.0.0" } : {}),
      source: "registry",
    }));

  it("shows the count of installed recipes the catalogue has newer, on the entry", async () => {
    mockApi({
      "GET /api/v1/plugins": {
        body: catalogue(
          [
            ["a", "1.0.0"],
            ["b", "1.0.0"],
            ["c", "1.0.0"],
          ],
          ["a", "c"],
        ),
      },
    });
    show("plugins");
    expect((await screen.findByTestId("plugins-badge")).textContent).toBe("2");
    const entry = screen.getByRole("link", { name: /Plugins/ });
    expect(entry.getAttribute("aria-current")).toBe("page");
    expect(entry.textContent).toBe("Plugins2");
  });

  it("shows no badge when nothing is newer or the read fails", async () => {
    const api = mockApi({
      "GET /api/v1/plugins": { body: catalogue([["a", "1.0.0"]], []) },
    });
    show("plugins");
    await waitFor(() => expect(api.count("GET /api/v1/plugins")).toBe(1));
    expect(screen.queryByTestId("plugins-badge")).toBeNull();
    cleanup();
    const failing = mockApi({ "GET /api/v1/plugins": { status: 500 } });
    show("plugins");
    await waitFor(() => expect(failing.count("GET /api/v1/plugins")).toBe(1));
    expect(screen.getByRole("link", { name: "Plugins" })).toBeTruthy();
    expect(screen.queryByTestId("plugins-badge")).toBeNull();
  });
});

describe("Sidebar, Log out", () => {
  it("sends the logout, clears the tokens and ends the session", async () => {
    storeSession("access-0", "refresh-0");
    useAuth.setState({ isAuthenticated: true });
    const api = mockApi({ "POST /api/v1/auth/logout": { status: 204 } });
    show();
    fireEvent.click(screen.getByRole("button", { name: "Log out" }));
    await waitFor(() => expect(useAuth.getState().isAuthenticated).toBe(false));
    expect(api.calls.find((c) => c.key === "POST /api/v1/auth/logout")).toEqual({
      key: "POST /api/v1/auth/logout",
      body: { refreshToken: "refresh-0" },
    });
    expect(localStorage.getItem("sowel_access_token")).toBeNull();
    expect(localStorage.getItem("sowel_refresh_token")).toBeNull();
  });
});
