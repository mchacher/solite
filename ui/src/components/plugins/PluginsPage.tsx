// Ported from Sowel ui/src/pages/PluginsPage.tsx (Sowel commit 5da84dc1, UI 1.87.0), spec 042, R5.4.
// Cut to script recipes: Sowel's category filter (Integrations / Recipes), the integration rows
// (status, devices, enable and disable), the detail sheet, the personal and community
// confirmations, the personal sources section, the "update all" banner and the registry's refresh
// button are cut (Solite installs no integration; the catalogue is read when the page opens; Remove
// stays on the row, with Sowel's two-step confirmation, as there is no detail sheet). Kept:
// the Installed and Store tabs with their counts, the search field, the row look (`card`), Install,
// Update and Remove. Added: the files refused at start, the
// error of the last read, and the server's own words for a refusal. The recipes are Sowel's
// `/plugins` routes (spec 059): the installed ones, the catalogue's not installed ones, and the
// catalogue's read date and error, a refusal at start and the files' rows from `/system/resources`;
// a 429 at opening leaves the list held.
import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { ArrowUpCircle, Download, Loader2, Package, Search, SearchX, X } from "lucide-react";
import {
  getPlugins,
  getPluginStore,
  getResources,
  installRecipe,
  refreshPluginStore,
  uninstallRecipe,
  updateRecipe,
  type PluginInfo,
  type Resources,
  type ScriptResource,
  type StoreEntry,
} from "../../api";
import { ApiError } from "../../api/client";
import { formErrorText } from "../../lib/form-error";
import { noteRecipePlugins } from "../layout/useRecipeUpdates";
import { BUTTON_SECONDARY } from "../ui/Modal";

type Tab = "installed" | "store";

/** The server's own words for a refusal (`Not enough memory for a: ...`), else the generic text. */
function serverText(err: unknown, t: Parameters<typeof formErrorText>[1]): string {
  if (err instanceof ApiError && err.fromServer && err.status >= 400 && err.message) {
    return err.message;
  }
  return formErrorText(err, t);
}

/** What a row shows of a recipe, installed or not. */
interface Row {
  id: string;
  name: string;
  description: string;
  version: string;
  /** The catalogue's newer version, set for an installed recipe that has one. */
  latest?: string;
  repo: string;
  /** The catalogue says this firmware cannot run it (`compatible: false`). */
  incompatible?: boolean;
}

const installedOf = (plugin: PluginInfo): Row => ({
  id: plugin.manifest.id,
  name: plugin.manifest.name,
  description: plugin.manifest.description,
  version: plugin.manifest.version,
  latest: plugin.latestVersion,
  repo: plugin.manifest.repo,
});

const storeOf = (entry: StoreEntry): Row => ({
  id: entry.id,
  name: entry.name,
  description: entry.description,
  version: entry.version,
  repo: entry.repo,
  incompatible: !entry.compatible,
});

const matches = (entry: Row, query: string) =>
  `${entry.id} ${entry.name} ${entry.description}`
    .toLowerCase()
    .includes(query.trim().toLowerCase());

export function PluginsPage() {
  const { t } = useTranslation();
  const [plugins, setPlugins] = useState<PluginInfo[] | null>(null);
  const [store, setStore] = useState<StoreEntry[]>([]);
  /** The catalogue's last read and why it failed (`GET /system/resources`). */
  const [catalogue, setCatalogue] = useState<Resources["catalogue"] | null>(null);
  /** The files refused at start (`GET /system/resources`). */
  const [files, setFiles] = useState<ScriptResource[]>([]);
  const [tab, setTab] = useState<Tab>("installed");
  const [query, setQuery] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  /** The recipe whose Remove was clicked once: the next click uninstalls. */
  const [confirming, setConfirming] = useState<string | null>(null);
  /** The recipes of the host's own scripts (`--recipe-script`): they cannot be removed. */
  const [builtIn, setBuiltIn] = useState<string[]>([]);
  const [message, setMessage] = useState<{ text: string; error: boolean } | null>(null);
  const alive = useRef(true);

  /** The installed recipes and the store as the box holds them now. */
  const readHeld = useCallback(async () => {
    const [installed, entries] = await Promise.all([getPlugins(), getPluginStore()]);
    if (!alive.current) return;
    setPlugins(installed);
    setStore(entries);
    noteRecipePlugins(installed);
  }, []);

  const readFiles = useCallback(
    () =>
      getResources().then(
        (r) => {
          if (!alive.current) return;
          setFiles(r.scripts.filter((s) => s.state === "refused"));
          setBuiltIn(r.scripts.filter((s) => s.source === "builtIn").map((s) => s.id));
          setCatalogue(r.catalogue);
        },
        () => undefined,
      ),
    [],
  );

  useEffect(() => {
    alive.current = true;
    let read = false;
    // What the box holds first, then the read: a 429 (within a minute of the last) keeps the list.
    void readHeld().then(
      () => undefined,
      (err: unknown) =>
        alive.current && !read && setMessage({ text: serverText(err, t), error: true }),
    );
    void readFiles();
    void refreshPluginStore().then(
      async () => {
        if (!alive.current) return;
        read = true;
        setMessage(null);
        try {
          await readHeld();
        } catch {
          // The list stays as it was.
        }
        await readFiles();
      },
      (err: unknown) => {
        // A 429 (paced) keeps the held list silently; any other refusal says why.
        if (alive.current && !(err instanceof ApiError && err.status === 429)) {
          setMessage({ text: serverText(err, t), error: true });
        }
      },
    );
    return () => {
      alive.current = false;
    };
  }, []);

  /** One action, then the list as the box holds it now. */
  const act = async (id: string, action: () => Promise<void>, done: string) => {
    setBusy(id);
    setConfirming(null);
    setMessage(null);
    try {
      await action();
      if (!alive.current) return;
      setMessage({ text: done, error: false });
    } catch (err) {
      if (!alive.current) return;
      setMessage({ text: serverText(err, t), error: true });
    }
    if (!alive.current) return;
    try {
      await readHeld();
    } catch {
      // The list stays as it was.
    }
    await readFiles();
    if (alive.current) setBusy(null);
  };

  const installedRows = (plugins ?? []).map(installedOf);
  const searching = query.trim().length > 0;
  const installed = installedRows.filter((r) => matches(r, query));
  // Files refused at start have no row of the installed list: by their id and their reason.
  const others = files.filter((f) =>
    `${f.id} ${f.reason ?? ""}`.toLowerCase().includes(query.trim().toLowerCase()),
  );
  const listed = store.map(storeOf).filter((r) => matches(r, query));
  const installedCount = installedRows.length + files.length;

  const removeButton = (id: string) => (
    <button
      type="button"
      disabled={busy !== null}
      onClick={() => {
        if (confirming !== id) {
          setConfirming(id);
          return;
        }
        void act(id, () => uninstallRecipe(id), t("recipeStore.removed"));
      }}
      onBlur={() => setConfirming((c) => (c === id ? null : c))}
      className={
        confirming === id
          ? "px-4 py-2 text-small font-medium rounded-sm transition-colors cursor-pointer disabled:opacity-50 text-on-status bg-error"
          : BUTTON_SECONDARY
      }
    >
      {confirming === id ? t("plugins.uninstallConfirm") : t("recipeStore.remove")}
    </button>
  );

  const updateButton = (entry: Row) => (
    <button
      type="button"
      disabled={busy !== null}
      onClick={() => act(entry.id, () => updateRecipe(entry.id), t("recipeStore.updated"))}
      className="flex items-center gap-1 px-2.5 py-1 text-micro font-medium rounded-sm transition-colors cursor-pointer disabled:opacity-50 text-error hover:bg-error/10"
    >
      {busy === entry.id ? (
        <Loader2 size={14} className="animate-spin" />
      ) : (
        <>
          <ArrowUpCircle size={12} />
          {t("recipeStore.update")}
        </>
      )}
    </button>
  );

  const identity = (entry: Row, detail: ReactNode) => (
    <div className="flex items-center gap-3 min-w-0 flex-1 px-4 py-3">
      <div className="w-9 h-9 bg-border-light rounded-md flex items-center justify-center shrink-0">
        <Package size={18} className="text-text-secondary" />
      </div>
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2 min-w-0 flex-wrap">
          <span className="text-body font-semibold text-text truncate">{entry.name}</span>
          <span className="text-micro px-1.5 py-0.5 bg-border-light rounded-sm text-text-tertiary font-mono shrink-0">
            {entry.version}
          </span>
        </div>
        <p className="text-caption text-text-secondary mt-0.5 line-clamp-2 sm:line-clamp-1">
          {entry.description}
        </p>
        {detail}
      </div>
    </div>
  );

  const installedRow = (entry: Row) => (
    <div
      key={entry.id}
      data-testid={`recipe-${entry.id}`}
      className="card flex flex-wrap items-center"
    >
      {identity(entry, null)}
      <div className="flex items-center justify-end gap-1 shrink-0 basis-full sm:basis-auto px-4 pb-3 sm:pb-0">
        {entry.latest && (
          <span className="text-micro px-1.5 py-0.5 bg-error/10 rounded-sm text-error font-mono shrink-0 flex items-center gap-0.5">
            <ArrowUpCircle size={10} />
            {entry.latest}
          </span>
        )}
        {entry.latest && updateButton(entry)}
        {!builtIn.includes(entry.id) && removeButton(entry.id)}
      </div>
    </div>
  );

  const otherRow = (file: ScriptResource) => (
    <div
      key={file.id}
      data-testid={`refused-${file.id}`}
      className="card flex flex-wrap items-center"
    >
      <div className="flex items-center gap-3 min-w-0 flex-1 px-4 py-3">
        <div className="w-9 h-9 bg-border-light rounded-md flex items-center justify-center shrink-0">
          <Package size={18} className="text-text-secondary" />
        </div>
        <div className="min-w-0 flex-1">
          <span className="text-body font-semibold text-text truncate">{file.id}</span>
          {file.version && (
            <span className="ml-2 text-micro px-1.5 py-0.5 bg-border-light rounded-sm text-text-tertiary font-mono">
              {file.version}
            </span>
          )}
          {file.state === "refused" && (
            <p className="text-micro text-error">
              {t("recipeStore.refused", { reason: file.reason ?? "" })}
            </p>
          )}
        </div>
      </div>
      <div className="flex justify-end shrink-0 basis-full sm:basis-auto px-4 pb-3 sm:pb-0">
        {removeButton(file.id)}
      </div>
    </div>
  );

  /** Why a file of that id was refused at start, if one was. */
  const refusedOf = (id: string) => files.find((f) => f.id === id)?.reason ?? null;

  /** The reason the catalogue's entry gives (`Requires Solite >= x`). */
  const incompatibleText = (id: string) => store.find((e) => e.id === id)?.compatReason ?? "";

  const storeRow = (entry: Row) => (
    <div
      key={entry.id}
      data-testid={`recipe-${entry.id}`}
      className="card flex flex-wrap items-center"
    >
      {identity(
        entry,
        refusedOf(entry.id) && (
          <p className="text-micro text-error">
            {t("recipeStore.refused", { reason: refusedOf(entry.id) ?? "" })}
          </p>
        ),
      )}
      <div className="flex items-center justify-end gap-1 shrink-0 basis-full sm:basis-auto px-4 pb-3 sm:pb-0">
        {entry.incompatible && (
          <span data-testid={`incompatible-${entry.id}`} className="text-micro text-error">
            {incompatibleText(entry.id)}
          </span>
        )}
        <button
          type="button"
          disabled={busy !== null || entry.incompatible}
          onClick={() => act(entry.id, () => installRecipe(entry.repo), t("recipeStore.installed"))}
          className="flex items-center gap-1.5 px-3 py-1.5 text-caption font-medium text-white bg-primary-fill hover:bg-primary-fill-hover rounded-sm transition-colors cursor-pointer disabled:opacity-50 disabled:cursor-not-allowed"
        >
          {busy === entry.id ? (
            <Loader2 size={14} className="animate-spin" />
          ) : (
            <Download size={14} />
          )}
          {t("recipeStore.install")}
        </button>
      </div>
    </div>
  );

  const noResults = (
    <div className="text-center py-16 text-text-tertiary text-body">
      <SearchX size={40} strokeWidth={1} className="mx-auto mb-3 text-text-tertiary/50" />
      {t("plugins.search.noResults", { query: query.trim() })}
    </div>
  );

  const empty = (text: string) => (
    <div className="text-center py-16 text-text-tertiary text-body">
      <Package size={40} strokeWidth={1} className="mx-auto mb-3 text-text-tertiary/50" />
      {text}
    </div>
  );

  if (plugins === null && message === null) {
    return (
      <div className="px-4 sm:px-6 flex items-center justify-center py-20">
        <Loader2 size={20} className="animate-spin text-text-tertiary" />
      </div>
    );
  }

  return (
    <div className="p-4 sm:p-6">
      <div className="max-w-[720px]">
        <div className="mb-5 flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
          <div>
            <h1 className="text-display font-semibold text-text">{t("plugins.title")}</h1>
            <p className="text-caption text-text-tertiary mt-1">{t("recipeStore.description")}</p>
          </div>
        </div>

        <div role="tablist" className="flex gap-1 mb-4">
          <TabButton
            active={tab === "installed"}
            onClick={() => setTab("installed")}
            label={t("plugins.installed")}
            count={installedCount}
          />
          <TabButton
            active={tab === "store"}
            onClick={() => setTab("store")}
            label={t("plugins.store")}
            count={store.length}
          />
        </div>

        <div className="relative mb-4">
          <Search
            size={15}
            strokeWidth={1.5}
            className="absolute left-3 top-1/2 -translate-y-1/2 text-text-tertiary pointer-events-none"
          />
          <input
            value={query}
            onChange={(e) => setQuery(e.currentTarget.value)}
            placeholder={t("plugins.search.placeholder")}
            aria-label={t("plugins.search.placeholder")}
            spellcheck={false}
            className="w-full pl-9 pr-9 py-2 text-small text-text border border-border bg-surface rounded-sm placeholder:text-text-tertiary focus:outline-none focus:border-primary transition-colors"
          />
          {searching && (
            <button
              type="button"
              onClick={() => setQuery("")}
              title={t("common.clear")}
              aria-label={t("common.clear")}
              className="absolute right-2 top-1/2 -translate-y-1/2 p-1 text-text-tertiary hover:text-text rounded-sm transition-colors cursor-pointer"
            >
              <X size={14} strokeWidth={1.5} />
            </button>
          )}
        </div>

        {message && (
          <p
            role={message.error ? "alert" : "status"}
            className={`text-small mb-3 ${message.error ? "text-error" : "text-success"}`}
          >
            {message.text}
          </p>
        )}

        {tab === "installed" ? (
          installed.length + others.length === 0 ? (
            searching ? (
              noResults
            ) : (
              empty(t("recipeStore.noneInstalled"))
            )
          ) : (
            <div className="space-y-2">
              {installed.map(installedRow)}
              {others.map(otherRow)}
            </div>
          )
        ) : (
          <div className="space-y-2">
            {catalogue?.error && (
              <p data-testid="read-error" className="text-small text-error mb-3">
                {t("recipeStore.readError", { reason: catalogue.error })}
              </p>
            )}
            {listed.length === 0
              ? searching
                ? noResults
                : !catalogue?.error && catalogue?.readAt && empty(t("recipeStore.empty"))
              : listed.map(storeRow)}
          </div>
        )}
      </div>
    </div>
  );
}

function TabButton({
  active,
  onClick,
  label,
  count,
}: {
  active: boolean;
  onClick: () => void;
  label: string;
  count: number;
}) {
  return (
    <button
      type="button"
      role="tab"
      aria-selected={active}
      onClick={onClick}
      className={`px-4 py-2 text-small font-medium rounded-sm transition-colors cursor-pointer ${
        active
          ? "bg-primary-fill text-white"
          : "border border-border bg-surface text-text-secondary hover:bg-border-light hover:text-text"
      }`}
    >
      {label}
      <span className={`ml-1.5 text-micro ${active ? "text-white/70" : "text-text-tertiary"}`}>
        {count}
      </span>
    </button>
  );
}
