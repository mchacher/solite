import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/preact";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  aggregate,
  homeRoutes,
  mockApi,
  resetStores,
  storeSession,
  zone,
  type Route,
} from "../../test-utils";
import { useAuth } from "../../store/useAuth";
import { useZoneAggregation } from "../../store/useZoneAggregation";
import { SettingsPage } from "./SettingsPage";
import { App } from "../../App";

const ROOT = "00000000-0000-0000-0000-000000000001";
const VERSION = { current: "0.1.0", latest: null, updateAvailable: false, releaseUrl: null };
const ZONES = ["UTC", "Europe/Paris", "America/New_York"];

beforeEach(() => {
  window.location.hash = "";
});

afterEach(async () => {
  vi.useRealTimers();
  cleanup();
  resetStores();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  await new Promise((resolve) => setTimeout(resolve, 120));
});

async function setup(settings: Record<string, string> = {}, routes: Record<string, Route> = {}) {
  const api = mockApi({
    "GET /api/v1/settings": { body: settings },
    "GET /api/v1/settings/timezones": { body: ZONES },
    "GET /api/v1/zones": { body: [zone(ROOT, "Home")] },
    "GET /api/v1/zones/aggregation": { body: {} },
    "GET /api/v1/system/version": { body: VERSION },
    ...routes,
  });
  render(<SettingsPage />);
  await screen.findByLabelText("Latitude");
  return api;
}

const field = (label: string) => screen.getByLabelText(label) as HTMLInputElement;
const save = () => screen.queryByRole("button", { name: "Save" });

describe("SettingsPage, Home card", () => {
  it("fills the fields and the time zone list, Save hidden", async () => {
    await setup({
      "home.name": "Annex",
      "home.latitude": "48.8566",
      "home.longitude": "2.3522",
      "home.timezone": "Europe/Paris",
      "home.sunriseOffset": "10",
    });
    expect(field("Home name").value).toBe("Annex");
    expect(field("Latitude").value).toBe("48.8566");
    expect(field("Longitude").value).toBe("2.3522");
    expect(field("Sunrise offset (min)").value).toBe("10");
    expect(field("Sunset offset (min)").value).toBe("45");
    const select = screen.getByLabelText("Timezone") as HTMLSelectElement;
    expect(select.value).toBe("Europe/Paris");
    expect([...select.options].map((o) => o.value)).toEqual(ZONES);
    expect(save()).toBeNull();
  });

  it("shows UTC when the time zone is unset, and saves the six keys with a comma turned to a dot", async () => {
    const api = await setup({}, { "PUT /api/v1/settings": { body: { success: true } } });
    expect((screen.getByLabelText("Timezone") as HTMLSelectElement).value).toBe("UTC");
    fireEvent.input(field("Latitude"), { target: { value: "48,8566" } });
    expect(field("Latitude").value).toBe("48.8566");
    fireEvent.change(screen.getByLabelText("Timezone"), { target: { value: "Europe/Paris" } });
    fireEvent.click(save()!);
    // The button reads "Saving..." while the GET then the PUT (saveSettings) run.
    await waitFor(() => expect(api.count("PUT /api/v1/settings")).toBe(1));
    await waitFor(() => expect(save()).toBeNull());
    expect(api.calls.find((c) => c.key === "PUT /api/v1/settings")!.body).toEqual({
      "home.name": "",
      "home.latitude": "48.8566",
      "home.longitude": "",
      "home.sunriseOffset": "30",
      "home.sunsetOffset": "45",
      "home.timezone": "Europe/Paris",
    });
  });

  it("reads the home settings and the time zone again after a successful save, for the top bar", async () => {
    const api = await setup(
      {},
      {
        "PUT /api/v1/settings": { body: { success: true } },
        "GET /api/v1/system/timezone": {
          body: { tz: "Europe/Paris", source: "env", offsetHours: 1 },
        },
      },
    );
    expect(api.count("GET /api/v1/system/timezone")).toBe(0);
    const settingsReads = api.count("GET /api/v1/settings"); // the cards' own reads
    fireEvent.input(field("Home name"), { target: { value: "Annex" } });
    fireEvent.click(save()!);
    await waitFor(() => expect(save()).toBeNull());
    await waitFor(() => expect(api.count("GET /api/v1/system/timezone")).toBe(1));
    // saveSettings' read just before the PUT (spec 040, R4.10), and the top bar's read after it.
    expect(api.count("GET /api/v1/settings")).toBe(settingsReads + 2);
  });

  it("shows the host's text when the PUT is refused, Save stays", async () => {
    await setup(
      {},
      {
        "PUT /api/v1/settings": { status: 400, body: { error: "Invalid value for home.latitude" } },
      },
    );
    fireEvent.input(field("Latitude"), { target: { value: "99x" } });
    fireEvent.click(save()!);
    expect(await screen.findByText("Invalid value for home.latitude")).toBeTruthy();
    expect(save()).toBeTruthy();
  });

  it("shows today's sun times and Day from the root aggregation, the hint when there are none", async () => {
    await setup();
    expect(screen.getByText("Set the location to get the sun times")).toBeTruthy();
    act(() => {
      useZoneAggregation.setState({
        data: { [ROOT]: aggregate({ sunrise: "05:46", sunset: "21:57", isDaylight: true }) },
      });
    });
    expect(screen.getByText("Sunrise 05:46 · Sunset 21:57 · Day")).toBeTruthy();
    act(() => {
      useZoneAggregation.setState({
        data: { [ROOT]: aggregate({ sunrise: "--:--", sunset: "--:--", isDaylight: false }) },
      });
    });
    expect(screen.getByText("Sunrise --:-- · Sunset --:-- · Night")).toBeTruthy();
  });

  it("speaks French in a French browser", async () => {
    vi.spyOn(navigator, "language", "get").mockReturnValue("fr-FR");
    const { setLanguage, detectLanguage } = await import("../../i18n");
    setLanguage(detectLanguage());
    try {
      mockApi({
        "GET /api/v1/settings": { body: {} },
        "GET /api/v1/settings/timezones": { body: ZONES },
      });
      render(<SettingsPage />);
      expect(await screen.findByLabelText("Fuseau horaire")).toBeTruthy();
      expect(screen.getByText("Aujourd'hui")).toBeTruthy();
      expect(screen.getByRole("button", { name: "Exporter" })).toBeTruthy();
    } finally {
      setLanguage("en");
    }
  });
});

const GET = "GET /api/v1/backup";
const POST = "POST /api/v1/backup";
const PASSWORD = "correct horse";

const exportButton = () => screen.getByRole("button", { name: /^Export/ }) as HTMLButtonElement;
const restoreButton = () => screen.getByRole("button", { name: /^Restor/ }) as HTMLButtonElement;
const typeIn = (label: string, value: string) =>
  fireEvent.input(screen.getByLabelText(label), { target: { value } });
const fillExport = (password = PASSWORD, confirm = PASSWORD) => {
  typeIn("Backup password", password);
  typeIn("Confirm the backup password", confirm);
};
const slb = () => new File(["SOLITEBK"], "box.slb");
const chooseBackup = (file = slb(), password = PASSWORD) => {
  fireEvent.change(screen.getByLabelText("Backup file (.slb)"), { target: { files: [file] } });
  typeIn("Password of the backup", password);
};

describe("SettingsPage, Backup card", () => {
  it("keeps Export disabled until the two passwords are equal and 8 to 128 characters, with the reason", async () => {
    await setup({}, { [GET]: { body: {} } });
    expect(exportButton().disabled).toBe(true);
    fillExport("short", "short");
    expect(exportButton().disabled).toBe(true);
    expect(screen.getByText("Password must be at least 8 characters")).toBeTruthy();
    fillExport(PASSWORD, "correct horsE");
    expect(exportButton().disabled).toBe(true);
    expect(screen.getByText("Passwords do not match")).toBeTruthy();
    fillExport("x".repeat(129), "x".repeat(129));
    expect(exportButton().disabled).toBe(true);
    expect(screen.getByText("Password must be at most 128 characters")).toBeTruthy();
    fillExport("x".repeat(128), "x".repeat(128));
    expect(exportButton().disabled).toBe(false);
  });

  it("exports: GET with the token and the percent-encoded password header, then a .slb download named by the date", async () => {
    storeSession();
    const api = await setup({}, { [GET]: { body: {} } });
    const created: Blob[] = [];
    vi.stubGlobal("URL", {
      createObjectURL: vi.fn((b: Blob) => (created.push(b), "blob:x")),
      revokeObjectURL: vi.fn(),
    });
    const names: string[] = [];
    const click = vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(function (
      this: HTMLAnchorElement,
    ) {
      names.push(this.download);
    });
    const password = "mot de passe é 😀";
    fillExport(password, password);
    fireEvent.click(exportButton());
    await waitFor(() => expect(click).toHaveBeenCalled());
    expect(api.count(GET)).toBe(1);
    const at = api.calls.findIndex((c) => c.key === GET);
    expect(api.headersOf[at]!["Authorization"]).toMatch(/^Bearer /);
    expect(api.headersOf[at]!["X-Backup-Password"]).toBe(encodeURIComponent(password));
    expect(created).toHaveLength(1);
    expect(created[0]).toBeInstanceOf(Blob);
    expect(names[0]).toMatch(/^solite-backup-\d{4}-\d{2}-\d{2}\.slb$/);
  });

  it("shows the host's text when the export is refused", async () => {
    await setup({}, { [GET]: { status: 409, body: { error: "Backup already in progress" } } });
    fillExport();
    fireEvent.click(exportButton());
    expect(await screen.findByText("Backup already in progress")).toBeTruthy();
  });

  it("shows the server's text for a 503 on the export and a 500 on the restore", async () => {
    await setup(
      {},
      {
        [GET]: { status: 503, body: { error: "Busy saving, try again" } },
        [POST]: { status: 500, body: { error: "Restore failed: no space left" } },
      },
    );
    fillExport();
    fireEvent.click(exportButton());
    // A GET answered 503 is retried once after 1 s by the client.
    expect(await screen.findByText("Busy saving, try again", {}, { timeout: 4000 })).toBeTruthy();
    chooseBackup();
    await waitFor(() => expect(restoreButton().disabled).toBe(false));
    fireEvent.click(restoreButton());
    fireEvent.click(
      within(await screen.findByRole("dialog")).getByRole("button", { name: "Restore" }),
    );
    expect(await screen.findByText("Restore failed: no space left")).toBeTruthy();
  });

  it("keeps Restore disabled until a file and a password are given", async () => {
    await setup();
    expect(restoreButton().disabled).toBe(true);
    fireEvent.change(screen.getByLabelText("Backup file (.slb)"), { target: { files: [slb()] } });
    await waitFor(() => expect(restoreButton().disabled).toBe(true));
    typeIn("Password of the backup", PASSWORD);
    await waitFor(() => expect(restoreButton().disabled).toBe(false));
  });

  it("restores after the confirmation: POST of the file as octet-stream with the token and the password, the message shown", async () => {
    storeSession();
    const api = await setup(
      {},
      {
        [POST]: {
          body: {
            success: true,
            message: "Backup restored: the box restarts, log in again after it",
            files: 5,
            wifi: false,
          },
        },
      },
    );
    const file = slb();
    chooseBackup(file);
    await waitFor(() => expect(restoreButton().disabled).toBe(false));
    fireEvent.click(restoreButton());
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText(/The account is kept/)).toBeTruthy();
    expect(api.count(POST)).toBe(0);
    fireEvent.click(within(dialog).getByRole("button", { name: "Restore" }));
    expect(
      await screen.findByText("Backup restored: the box restarts, log in again after it"),
    ).toBeTruthy();
    const at = api.calls.findIndex((c) => c.key === POST);
    expect(api.headersOf[at]!["Content-Type"]).toBe("application/octet-stream");
    expect(api.headersOf[at]!["Authorization"]).toMatch(/^Bearer /);
    expect(api.headersOf[at]!["X-Backup-Password"]).toBe(encodeURIComponent(PASSWORD));
    expect(api.calls[at]!.body).toBe(file);
  });

  it("asks for the Wi-Fi settings only when the box is ticked: ?wifi=true", async () => {
    const api = await setup(
      {},
      {
        "POST /api/v1/backup?wifi=true": {
          body: { success: true, message: "Backup restored", files: 6, wifi: true },
        },
      },
    );
    chooseBackup();
    fireEvent.click(screen.getByLabelText("Also restore the Wi-Fi settings"));
    await waitFor(() => expect(restoreButton().disabled).toBe(false));
    fireEvent.click(restoreButton());
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("The Wi-Fi settings are replaced too.")).toBeTruthy();
    fireEvent.click(within(dialog).getByRole("button", { name: "Restore" }));
    expect(await screen.findByText("Backup restored")).toBeTruthy();
    expect(api.count("POST /api/v1/backup?wifi=true")).toBe(1);
    expect(api.count(POST)).toBe(0);
  });

  it("sends nothing when the confirmation is cancelled", async () => {
    const api = await setup({}, { [POST]: { body: {} } });
    chooseBackup();
    await waitFor(() => expect(restoreButton().disabled).toBe(false));
    fireEvent.click(restoreButton());
    const dialog = await screen.findByRole("dialog");
    fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(api.count(POST)).toBe(0);
  });

  it("shows Restoring... and disables the button while the upload runs", async () => {
    let release: () => void = () => {};
    const gate = new Promise<void>((resolve) => (release = resolve));
    await setup(
      {},
      {
        [POST]: async () => {
          await gate;
          return { body: { success: true, message: "Backup restored", files: 1, wifi: false } };
        },
      },
    );
    chooseBackup();
    await waitFor(() => expect(restoreButton().disabled).toBe(false));
    fireEvent.click(restoreButton());
    fireEvent.click(
      within(await screen.findByRole("dialog")).getByRole("button", { name: "Restore" }),
    );
    const busy = await screen.findByRole("button", { name: "Restoring..." });
    expect((busy as HTMLButtonElement).disabled).toBe(true);
    release();
    expect(await screen.findByText("Backup restored")).toBeTruthy();
  });

  it("shows the host's text when the restore is refused (wrong password)", async () => {
    await setup({}, { [POST]: { status: 400, body: { error: "Wrong backup password" } } });
    chooseBackup();
    await waitFor(() => expect(restoreButton().disabled).toBe(false));
    fireEvent.click(restoreButton());
    fireEvent.click(
      within(await screen.findByRole("dialog")).getByRole("button", { name: "Restore" }),
    );
    expect(await screen.findByText("Wrong backup password")).toBeTruthy();
  });

  it("shows the hint of a confirmRequired refusal", async () => {
    await setup(
      {},
      {
        [POST]: {
          status: 403,
          body: { error: "Press the button on the box, then try again", confirmRequired: true },
        },
      },
    );
    chooseBackup();
    await waitFor(() => expect(restoreButton().disabled).toBe(false));
    fireEvent.click(restoreButton());
    fireEvent.click(
      within(await screen.findByRole("dialog")).getByRole("button", { name: "Restore" }),
    );
    expect(await screen.findByText("Press the button on the box, then try again")).toBeTruthy();
  });
});

describe("Settings in the navigation", () => {
  it("has its entry in Administration, and #/settings shows the page", async () => {
    mockApi({
      ...homeRoutes({ zones: [zone(ROOT, "Home")] }),
      "GET /api/v1/settings": { body: {} },
      "GET /api/v1/settings/timezones": { body: ZONES },
    });
    window.location.hash = "#/settings";
    storeSession();
    render(<App />);
    expect(await screen.findByRole("heading", { level: 1, name: "Settings" })).toBeTruthy();
    const link = screen.getAllByRole("link", { name: "Settings" })[0];
    expect(link.getAttribute("href")).toBe("#/settings");
    expect(link.getAttribute("aria-current")).toBeTruthy();
  });
});

describe("SettingsPage, factory reset card", () => {
  const open = () => fireEvent.click(screen.getByRole("button", { name: "Factory reset" }));
  const RESET = "POST /api/v1/system/factory-reset";

  it("asks for a confirmation first, and Cancel sends nothing", async () => {
    const api = await setup();
    open();
    const dialog = screen.getByRole("dialog", { name: "Erase everything?" });
    fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(api.count(RESET)).toBe(0);
  });

  it("success: sends {} as JSON, clears the tokens and shows the first-start state", async () => {
    storeSession();
    useAuth.setState({ isAuthenticated: true, loading: false, setupRequired: false });
    const api = await setup({}, { [RESET]: { status: 204 } });
    open();
    fireEvent.click(screen.getByRole("button", { name: "Erase everything" }));
    await waitFor(() => expect(useAuth.getState().setupRequired).toBe(true));
    expect(api.calls.find((c) => c.key === RESET)!.body).toEqual({});
    expect(useAuth.getState().isAuthenticated).toBe(false);
    expect(localStorage.getItem("sowel_refresh_token")).toBeNull();
  });

  it("refusal: shows the host's text and keeps the session", async () => {
    storeSession();
    useAuth.setState({ isAuthenticated: true, loading: false, setupRequired: false });
    await setup(
      {},
      {
        [RESET]: {
          status: 403,
          body: { error: "Press the button on the box, then try again", confirmRequired: true },
        },
      },
    );
    open();
    fireEvent.click(screen.getByRole("button", { name: "Erase everything" }));
    expect(await screen.findByText("Press the button on the box, then try again")).toBeTruthy();
    expect(useAuth.getState().isAuthenticated).toBe(true);
    expect(localStorage.getItem("sowel_refresh_token")).toBe("refresh-0");
  });
});

describe("SettingsPage, Update card", () => {
  const sup = () => new File(["SUP"], "solite.sup");
  const choose = (file: File) =>
    fireEvent.change(screen.getByLabelText("Update file"), { target: { files: [file] } });
  const install = () => screen.getByRole("button", { name: /^Install/ }) as HTMLButtonElement;

  it("shows the running version", async () => {
    await setup();
    expect(await screen.findByText("Running version: 0.1.0")).toBeTruthy();
  });

  it("keeps Install disabled until a file is chosen", async () => {
    await setup();
    expect(install().disabled).toBe(true);
    choose(sup());
    await waitFor(() => expect(install().disabled).toBe(false));
  });

  it("posts the file as octet-stream with the token, shows the message and the reload line", async () => {
    storeSession();
    const api = await setup(
      {},
      { "POST /api/v1/system/update": { body: { message: "Installed 0.2.0, restarting" } } },
    );
    choose(sup());
    fireEvent.click(install());
    expect(await screen.findByText("Installed 0.2.0, restarting")).toBeTruthy();
    expect(screen.getByText("The box restarts; reload the page in a minute.")).toBeTruthy();
    const at = api.calls.findIndex((c) => c.key === "POST /api/v1/system/update");
    const call = api.calls[at]!;
    expect(api.headersOf[at]!["Content-Type"]).toBe("application/octet-stream");
    expect(api.headersOf[at]!["Authorization"]).toMatch(/^Bearer /);
    expect(call.body).toBeInstanceOf(File);
  });

  it("shows the error text of a refused file, no reload line", async () => {
    await setup(
      {},
      { "POST /api/v1/system/update": { status: 400, body: { error: "Bad signature" } } },
    );
    choose(sup());
    fireEvent.click(install());
    expect(await screen.findByText("Bad signature")).toBeTruthy();
    expect(screen.queryByText("The box restarts; reload the page in a minute.")).toBeNull();
  });

  it("shows Installing… and disables the button while the upload runs", async () => {
    let release: () => void = () => {};
    const gate = new Promise<void>((resolve) => (release = resolve));
    await setup(
      {},
      {
        "POST /api/v1/system/update": async () => {
          await gate;
          return { body: { message: "Installed 0.2.0, restarting" } };
        },
      },
    );
    choose(sup());
    fireEvent.click(install());
    const busy = await screen.findByRole("button", { name: "Installing…" });
    expect((busy as HTMLButtonElement).disabled).toBe(true);
    release();
    expect(await screen.findByText("Installed 0.2.0, restarting")).toBeTruthy();
  });
});

describe("SettingsPage, Update card, a release (spec 040, R4)", () => {
  const AVAILABLE = {
    current: "0.1.0",
    latest: "0.2.0",
    updateAvailable: true,
    releaseUrl: "https://example.test/releases/v0.2.0",
  };
  const NEW = { ...AVAILABLE, current: "0.2.0", updateAvailable: false };
  const VERSIONS = "GET /api/v1/system/version";
  const PULL = "POST /api/v1/system/update";

  /** What the box answers to a version read: an answer, a refusal, or no answer at all (a throw). */
  const box: { info: unknown; mode: "up" | "down" | "busy" } = { info: AVAILABLE, mode: "up" };
  const versionRoute: Route = () => {
    if (box.mode === "down") throw new TypeError("Failed to fetch");
    if (box.mode === "busy") return { status: 500, body: { error: "Busy" } };
    return { body: box.info };
  };

  beforeEach(() => {
    box.info = AVAILABLE;
    box.mode = "up";
  });

  const open = (routes: Record<string, Route | Route[]> = {}, settings = {}) =>
    setup(settings, { [VERSIONS]: versionRoute, ...routes });
  const tick = (ms: number) =>
    act(async () => {
      await vi.advanceTimersByTimeAsync(ms);
    });
  const button = (name: string | RegExp) =>
    screen.getByRole("button", { name }) as HTMLButtonElement;
  const confirmUpdate = async () => {
    fireEvent.click(await screen.findByRole("button", { name: "Update to v0.2.0" }));
    fireEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Update" }));
  };
  /** The box answers the pull with 200 at once; the polling starts after it. */
  const startPull = async (routes: Record<string, Route | Route[]> = {}) => {
    const api = await open({ [PULL]: { body: { message: "ok" } }, ...routes });
    await screen.findByText("Version 0.2.0 is available");
    vi.useFakeTimers();
    await confirmUpdate();
    await tick(0);
    return api;
  };

  it("shows the four states of the version line, the button and the link only when they apply", async () => {
    box.info = { ...AVAILABLE, latest: null, updateAvailable: false, releaseUrl: null };
    await open();
    expect(await screen.findByText("No check since the box started")).toBeTruthy();
    expect(screen.queryByRole("button", { name: /^Update to/ })).toBeNull();
    cleanup();

    box.info = AVAILABLE;
    await open();
    expect(await screen.findByText("Version 0.2.0 is available")).toBeTruthy();
    expect(button("Update to v0.2.0")).toBeTruthy();
    expect(screen.getByRole("link", { name: "Release notes" }).getAttribute("href")).toBe(
      "https://example.test/releases/v0.2.0",
    );
    expect(screen.getByRole("link", { name: "Release notes" }).getAttribute("target")).toBe(
      "_blank",
    );
    cleanup();

    box.info = { ...AVAILABLE, releaseUrl: null };
    await open();
    await screen.findByText("Version 0.2.0 is available");
    expect(screen.queryByRole("link", { name: "Release notes" })).toBeNull();
    cleanup();

    box.info = { ...AVAILABLE, latest: "0.10.0", updateAvailable: false };
    await open();
    expect(
      await screen.findByText(
        "Version 0.10.0 cannot be installed directly from this version: see its release notes",
      ),
    ).toBeTruthy();
    expect(screen.getByRole("link", { name: "Release notes" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: /^Update to/ })).toBeNull();
    cleanup();

    box.info = { ...AVAILABLE, latest: "0.1.0", updateAvailable: false };
    await open();
    expect(await screen.findByText("Up to date (latest 0.1.0)")).toBeTruthy();
  });

  it("checks: POST, a spinner while it runs, the card refreshed with the answer", async () => {
    let release: () => void = () => {};
    const gate = new Promise<void>((resolve) => (release = resolve));
    box.info = { ...AVAILABLE, latest: null, updateAvailable: false, releaseUrl: null };
    const api = await open({
      "POST /api/v1/system/version/check": async () => {
        await gate;
        return { body: AVAILABLE };
      },
    });
    await screen.findByText("No check since the box started");
    fireEvent.click(button("Check for updates"));
    await waitFor(() => expect(button("Check for updates").disabled).toBe(true));
    expect(document.querySelector(".animate-spin")).toBeTruthy();
    release();
    expect(await screen.findByText("Version 0.2.0 is available")).toBeTruthy();
    expect(api.count("POST /api/v1/system/version/check")).toBe(1);
    expect(button("Check for updates").disabled).toBe(false);
  });

  it.each([
    [429, "Please wait 12s before checking again"],
    [503, "The clock is not set yet: try again later"],
    [500, "Update check failed: timeout"],
    [409, "Update check already in progress"],
  ])("shows the server's own text for a check refused with %i", async (status, text) => {
    await open({ "POST /api/v1/system/version/check": { status, body: { error: text } } });
    fireEvent.click(await screen.findByRole("button", { name: "Check for updates" }));
    expect(await screen.findByText(text)).toBeTruthy();
  });

  it("opens a confirmation; Cancel sends nothing", async () => {
    const api = await open();
    fireEvent.click(await screen.findByRole("button", { name: "Update to v0.2.0" }));
    const dialog = screen.getByRole("dialog");
    expect(within(dialog).getByText("Update to v0.2.0?")).toBeTruthy();
    expect(within(dialog).getByText(/You will need to log in again\./)).toBeTruthy();
    fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(api.count(PULL)).toBe(0);
  });

  it("confirmed: POST with no body, the buttons disabled, the elapsed count advances", async () => {
    let release: () => void = () => {};
    const gate = new Promise<void>((resolve) => (release = resolve));
    const api = await open({
      [PULL]: async () => {
        await gate;
        return { body: { message: "ok" } };
      },
    });
    await screen.findByText("Version 0.2.0 is available");
    vi.useFakeTimers();
    await confirmUpdate();
    await tick(0);
    expect(api.calls.find((c) => c.key === PULL)!.body).toBeUndefined();
    expect(screen.getByText("Downloading and installing… 0 s")).toBeTruthy();
    expect(button("Check for updates").disabled).toBe(true);
    expect(button("Update to v0.2.0").disabled).toBe(true);
    await tick(3000);
    expect(screen.getByText("Downloading and installing… 3 s")).toBeTruthy();
    await tick(7000);
    expect(screen.getByText("Downloading and installing… 10 s")).toBeTruthy();
    release();
    await tick(0);
  });

  it("200, then the polling: two failed polls, then a new version: Now running", async () => {
    const api = await startPull();
    expect(screen.getByText("Installed 0.2.0, the box restarts…")).toBeTruthy();
    const before = api.count(VERSIONS);
    box.mode = "down";
    await tick(3000);
    expect(api.count(VERSIONS)).toBe(before + 1);
    await tick(3000);
    expect(api.count(VERSIONS)).toBe(before + 2);
    box.mode = "up";
    box.info = NEW;
    await tick(3000);
    expect(screen.getByText("Now running 0.2.0")).toBeTruthy();
    const after = api.count(VERSIONS);
    await tick(30_000);
    expect(api.count(VERSIONS)).toBe(after); // the polling stopped
    expect(button("Check for updates").disabled).toBe(false);
  });

  it("200, the box never answers for 3 minutes: the reload text, the polling stopped", async () => {
    const api = await startPull();
    box.mode = "down";
    await tick(170_000);
    expect(screen.queryByText(/not answered/)).toBeNull();
    await tick(10_000);
    expect(
      screen.getByText("The box has not answered for 3 minutes: reload the page"),
    ).toBeTruthy();
    const count = api.count(VERSIONS);
    await tick(60_000);
    expect(api.count(VERSIONS)).toBe(count);
  });

  it("200, the same version for 3 minutes: it was not kept", async () => {
    await startPull();
    await tick(170_000);
    expect(screen.queryByText(/was not kept/)).toBeNull();
    await tick(10_000);
    expect(
      screen.getByText(
        "The box still runs 0.1.0: the new version was not kept (see the box's log)",
      ),
    ).toBeTruthy();
  });

  it("3 minutes: the ending comes from the last poll (answered before, silent at the end)", async () => {
    await startPull();
    await tick(100_000); // answered with the same version until here
    box.mode = "down";
    await tick(80_000);
    expect(
      screen.getByText("The box has not answered for 3 minutes: reload the page"),
    ).toBeTruthy();
  });

  it("a dropped connection during the request: waits, then the same version at 11 minutes", async () => {
    const api = await startPull({
      [PULL]: () => {
        throw new TypeError("Failed to fetch");
      },
    });
    expect(screen.getByText("The connection was lost; waiting for the box…")).toBeTruthy();
    await tick(10 * 60_000);
    expect(screen.queryByText(/did not complete/)).toBeNull();
    expect(api.count(VERSIONS)).toBeGreaterThan(100);
    await tick(60_000);
    expect(screen.getByText("The update did not complete: the box still runs 0.1.0")).toBeTruthy();
  });

  it("a dropped connection, then a new version: Now running", async () => {
    await startPull({
      [PULL]: () => {
        throw new TypeError("Failed to fetch");
      },
    });
    await tick(60_000);
    box.info = NEW;
    await tick(3000);
    expect(screen.getByText("Now running 0.2.0")).toBeTruthy();
  });

  it("aborts a poll that hangs at 5 s, and never runs two polls at once", async () => {
    const api = await startPull();
    const real = vi.mocked(fetch);
    let open = 0;
    let most = 0;
    let aborted = 0;
    vi.stubGlobal(
      "fetch",
      vi.fn((url: string, init?: RequestInit) => {
        if (url !== "/api/v1/system/version") return real(url, init);
        open++;
        most = Math.max(most, open);
        return new Promise((_, reject) => {
          init?.signal?.addEventListener("abort", () => {
            open--;
            aborted++;
            reject(Object.assign(new Error("aborted"), { name: "AbortError" }));
          });
        });
      }),
    );
    const before = api.count(VERSIONS);
    await tick(3000); // a poll starts and hangs
    expect(open).toBe(1);
    await tick(4900);
    expect(aborted).toBe(0);
    await tick(200);
    expect(aborted).toBe(1);
    await tick(3000); // the next one starts 3 s after the previous ended
    expect(open).toBe(1);
    await tick(60_000);
    expect(most).toBe(1);
    expect(api.count(VERSIONS)).toBe(before);
  });

  it("stops polling when the card unmounts (the login screen)", async () => {
    const api = await startPull();
    await tick(3000);
    const count = api.count(VERSIONS);
    cleanup();
    await tick(60_000);
    expect(api.count(VERSIONS)).toBe(count);
  });

  it("disables the upload during a pull, and the pull and the check during an upload", async () => {
    await startPull();
    expect((screen.getByLabelText("Update file") as HTMLInputElement).disabled).toBe(true);
    cleanup();
    vi.useRealTimers();

    let release: () => void = () => {};
    const gate = new Promise<void>((resolve) => (release = resolve));
    await open({
      [PULL]: async () => {
        await gate;
        return { body: { message: "Installed 0.2.0, restarting" } };
      },
    });
    await screen.findByText("Version 0.2.0 is available");
    fireEvent.change(screen.getByLabelText("Update file"), {
      target: { files: [new File(["SUP"], "solite.sup")] },
    });
    fireEvent.click(await screen.findByRole("button", { name: "Install" }));
    await waitFor(() => expect(button("Update to v0.2.0").disabled).toBe(true));
    expect(button("Check for updates").disabled).toBe(true);
    release();
    await screen.findByText("Installed 0.2.0, restarting");
    expect(button("Check for updates").disabled).toBe(false);
  });

  it.each([
    [400, "No update available"],
    [409, "The running image is not confirmed yet"],
    [500, "Download failed: connection reset"],
    [503, "The clock is not set yet: try again later"],
  ])("shows the server's text for a pull refused with %i, the card back", async (status, text) => {
    const api = await open({ [PULL]: { status, body: { error: text } } });
    await screen.findByText("Version 0.2.0 is available");
    await confirmUpdate();
    expect(await screen.findByText(text)).toBeTruthy();
    expect(button("Update to v0.2.0").disabled).toBe(false);
    expect(button("Check for updates").disabled).toBe(false);
    expect(api.count(PULL)).toBe(1);
  });

  it("toggles the automatic check: the whole map is sent with update.autoCheck", async () => {
    const api = await open(
      { "PUT /api/v1/settings": { body: { success: true } } },
      { "home.name": "Annex", "update.manifestUrl": "http://x.test/m.json" },
    );
    const box_ = screen.getByLabelText("Check for a new version once a day") as HTMLInputElement;
    expect(box_.checked).toBe(false);
    fireEvent.click(box_);
    await waitFor(() => expect(api.count("PUT /api/v1/settings")).toBe(1));
    expect(api.calls.find((c) => c.key === "PUT /api/v1/settings")!.body).toEqual({
      "home.name": "Annex",
      "update.manifestUrl": "http://x.test/m.json",
      "update.autoCheck": "true",
    });
    await waitFor(() => expect(box_.checked).toBe(true));
  });

  it("saves the manifest's address with the whole map; an empty one returns to the default", async () => {
    const api = await open(
      { "PUT /api/v1/settings": { body: { success: true } } },
      { "home.name": "Annex", "update.autoCheck": "true" },
    );
    fireEvent.input(screen.getByLabelText("Manifest address"), {
      target: { value: "http://127.0.0.1:9/manifest.json" },
    });
    fireEvent.click(button("Save the manifest address"));
    await waitFor(() => expect(api.count("PUT /api/v1/settings")).toBe(1));
    expect(api.calls.find((c) => c.key === "PUT /api/v1/settings")!.body).toEqual({
      "home.name": "Annex",
      "update.autoCheck": "true",
      "update.manifestUrl": "http://127.0.0.1:9/manifest.json",
    });
  });

  it("shows the server's text when the manifest's address is refused", async () => {
    await open({
      "PUT /api/v1/settings": {
        status: 400,
        body: { error: "Invalid value for update.manifestUrl" },
      },
    });
    fireEvent.input(screen.getByLabelText("Manifest address"), { target: { value: "nope" } });
    fireEvent.click(button("Save the manifest address"));
    expect(await screen.findByText("Invalid value for update.manifestUrl")).toBeTruthy();
  });

  it("the Home card keeps the update.* keys it did not touch", async () => {
    const api = await setup(
      { "update.autoCheck": "true", "update.manifestUrl": "http://x.test/m.json" },
      { "PUT /api/v1/settings": { body: { success: true } } },
    );
    fireEvent.input(field("Home name"), { target: { value: "Annex" } });
    fireEvent.click(save()!);
    await waitFor(() => expect(api.count("PUT /api/v1/settings")).toBe(1));
    const body = api.calls.find((c) => c.key === "PUT /api/v1/settings")!.body as Record<
      string,
      string
    >;
    expect(body["update.autoCheck"]).toBe("true");
    expect(body["update.manifestUrl"]).toBe("http://x.test/m.json");
    expect(body["home.name"]).toBe("Annex");
  });
});
