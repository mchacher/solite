// The script recipes' store, at Sowel's `/plugins` routes (spec 059, D89; spec 042, D78): the
// installed ones, the catalogue, and the install, update and removal, and Solite's own
// `GET /system/resources`.
import { API_BASE, fetchJSON } from "./client";

/** Sowel's `PluginManifest` (`src/shared/types.ts`), as a script recipe has it. */
export interface PluginManifest {
  id: string;
  name: string;
  version: string;
  description: string;
  icon?: string;
  /** `owner/name`: the catalogue's, else `solite-recipes/<id>`. */
  repo: string;
  type: "recipe";
  category?: string;
  author?: string;
}

/** An installed script recipe: Sowel's `PluginInfo`, `GET /plugins`. */
export interface PluginInfo {
  manifest: PluginManifest;
  enabled: boolean;
  /** ISO 8601; `null` when the box does not know. */
  installedAt: string | null;
  status: string;
  deviceCount: number;
  offlineDeviceCount: number;
  /** Set when the held catalogue has a newer version. */
  latestVersion?: string;
  source: "registry" | "personal";
}

/** One recipe of the catalogue not installed: Sowel's store entry, `GET /plugins/store`. */
export interface StoreEntry extends PluginManifest {
  compatible: boolean;
  compatReason?: string;
  isOfficial: boolean;
  tier: "official" | "community" | "personal";
  tags?: string[];
}

/** What the box holds, never the network: the installed recipes. */
export async function getPlugins(): Promise<PluginInfo[]> {
  return fetchJSON<PluginInfo[]>(`${API_BASE}/plugins`);
}

/** What the box holds, never the network: the catalogue's recipes that are not installed. */
export async function getPluginStore(): Promise<StoreEntry[]> {
  return fetchJSON<StoreEntry[]>(`${API_BASE}/plugins/store`);
}

/** Reads the catalogue again (at most once a minute: a 429 leaves the list held). */
export async function refreshPluginStore(): Promise<{ count: number; source: "remote" | "local" }> {
  return fetchJSON<{ count: number; source: "remote" | "local" }>(
    `${API_BASE}/plugins/store/refresh`,
    { method: "POST" },
  );
}

export async function installRecipe(repo: string): Promise<void> {
  await fetchJSON<unknown>(`${API_BASE}/plugins/install`, {
    method: "POST",
    body: JSON.stringify({ repo }),
  });
}

export async function updateRecipe(id: string): Promise<void> {
  await fetchJSON<unknown>(`${API_BASE}/plugins/${encodeURIComponent(id)}/update`, {
    method: "POST",
  });
}

export async function uninstallRecipe(id: string): Promise<void> {
  await fetchJSON<unknown>(`${API_BASE}/plugins/${encodeURIComponent(id)}/uninstall`, {
    method: "POST",
  });
}

export interface ScriptResource {
  id: string;
  version: string | null;
  source: "builtIn" | "installed";
  state: "ready" | "failed" | "refused";
  reason: string | null;
  held: number;
  limit: number;
  instances: number;
  calls: number;
  averageUs: number;
  maxUs: number;
}

/** `GET /system/resources`, bytes and microseconds as numbers; `null` where the platform reads none. */
export interface Resources {
  internal: { free: number; minFree: number; largestBlock: number } | null;
  psram: { free: number; minFree: number; total: number; reserve: number } | null;
  storage: { used: number; total: number | null };
  scripts: ScriptResource[];
  /** The recipe catalogue's last read: when (ISO 8601, `null` before any) and why it failed. */
  catalogue: { readAt: string | null; error: string | null };
}

export async function getResources(): Promise<Resources> {
  return fetchJSON<Resources>(`${API_BASE}/system/resources`);
}
