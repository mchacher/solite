import { cleanup, render, screen, waitFor, within } from "@testing-library/preact";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Resources } from "../../api";
import { mockApi, resetStores } from "../../test-utils";
import { ResourcesCard } from "./ResourcesCard";

const RESOURCES = "GET /api/v1/system/resources";

afterEach(() => {
  cleanup();
  resetStores();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

const BOX: Resources = {
  internal: { free: 153_600, minFree: 92_160, largestBlock: 81_920 },
  psram: { free: 1_048_576, minFree: 900_000, total: 2_097_152, reserve: 524_288 },
  storage: { used: 40_960, total: 1_048_576 },
  scripts: [
    {
      id: "motion-light-script",
      version: "1.0.0",
      source: "installed",
      state: "ready",
      reason: null,
      held: 112_640,
      limit: 229_376,
      instances: 2,
      calls: 120,
      averageUs: 24_000,
      maxUs: 83_000,
    },
    {
      id: "broken",
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
    },
  ],
  catalogue: { readAt: "2026-11-27T10:00:00.000Z", error: null },
};

describe("ResourcesCard", () => {
  it("shows when the catalogue was read, and why the last read failed", async () => {
    mockApi({
      [RESOURCES]: {
        body: { ...BOX, catalogue: { readAt: BOX.catalogue.readAt, error: "no route to host" } },
      },
    });
    render(<ResourcesCard />);
    expect((await screen.findByTestId("catalogue-read")).textContent).toContain(
      "Recipe catalogue read:",
    );
    expect(screen.getByTestId("catalogue-error").textContent).toBe(
      "The catalogue could not be read: no route to host",
    );
  });

  it("shows the two memories as bars, the storage and a row per script recipe", async () => {
    mockApi({ [RESOURCES]: { body: BOX } });
    render(<ResourcesCard />);
    const psram = await screen.findByTestId("psram");
    expect(
      within(psram).getByText("1024 KB free of 2048 KB (lowest 879 KB, reserve 512 KB)"),
    ).toBeTruthy();
    // Used 1024 of 2048 KB: half the bar.
    expect(within(psram).getByRole("progressbar").getAttribute("aria-valuenow")).toBe("50");
    // The lowest free and the reserve are marked on it.
    expect(psram.querySelector('[data-marker="Lowest free since start"]')).toBeTruthy();
    expect(psram.querySelector('[data-marker="Reserve kept for installs"]')).toBeTruthy();
    const internal = screen.getByTestId("internal");
    expect(
      within(internal).getByText("150 KB free (lowest 90 KB, largest block 80 KB)"),
    ).toBeTruthy();
    expect(screen.getByTestId("storage").textContent).toBe("Storage: 40 KB used of 1024 KB");
    const row = screen.getByTestId("script-motion-light-script");
    expect(row.textContent).toContain("motion-light-script 1.0.0");
    expect(row.textContent).toContain("Running");
    expect(row.textContent).toContain("110 KB held of 224 KB");
    expect(row.textContent).toContain("2 instances");
    expect(row.textContent).toContain("120 calls, 24000 µs on average, 83000 µs at most");
    // A refused file: its reason, no memory or calls.
    const refused = screen.getByTestId("script-broken");
    expect(refused.textContent).toContain("Refused");
    expect(refused.textContent).toContain("Invalid signature");
    expect(refused.textContent).not.toContain("KB held");
  });

  it("hides the bars when the platform reads no memory, and the storage total when it has none", async () => {
    mockApi({
      [RESOURCES]: {
        body: { internal: null, psram: null, storage: { used: 2048, total: null }, scripts: [] },
      },
    });
    render(<ResourcesCard />);
    await waitFor(() => expect(screen.getByTestId("storage")).toBeTruthy());
    expect(screen.queryByTestId("psram")).toBeNull();
    expect(screen.queryByTestId("internal")).toBeNull();
    expect(screen.getByTestId("storage").textContent).toBe("Storage: 2 KB used");
    expect(screen.getByText("No script recipe.")).toBeTruthy();
  });

  it("shows the reason when the resources cannot be read", async () => {
    mockApi({ [RESOURCES]: { status: 500, body: { error: "Internal error" } } });
    render(<ResourcesCard />);
    expect(
      await screen.findByText("The server could not handle the request. Try again in a moment."),
    ).toBeTruthy();
    expect(screen.queryByTestId("psram")).toBeNull();
  });
});
