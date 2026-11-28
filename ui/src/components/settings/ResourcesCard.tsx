// The Resources card (spec 042, R5.3): the box's two memories as a bar each, the storage, and a row
// per script recipe. Solite's own (Sowel has no such card): Sowel's `card` and tokens. A row is
// hidden when its figure is `null` (the Mac's host reads no memory).
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { getResources, type Resources, type ScriptResource } from "../../api";
import { formErrorText } from "../../lib/form-error";

const kb = (bytes: number) => Math.round(bytes / 1024);
const percent = (part: number, whole: number) =>
  whole <= 0 ? 0 : Math.min(100, Math.max(0, (part / whole) * 100));

/** A bar: the filled part, and a marker for each line worth reading on it. */
function Bar({
  label,
  fill,
  markers,
  summary,
  testId,
}: {
  label: string;
  fill: number;
  markers: { at: number; label: string }[];
  summary: string;
  testId: string;
}) {
  return (
    <div data-testid={testId} className="mb-3">
      <div className="flex justify-between text-small text-text-secondary mb-1">
        <span>{label}</span>
        <span>{summary}</span>
      </div>
      <div className="relative h-2 rounded-full bg-border-light overflow-hidden">
        <div
          role="progressbar"
          aria-label={label}
          aria-valuenow={Math.round(fill)}
          aria-valuemin={0}
          aria-valuemax={100}
          className="h-full bg-primary"
          style={{ width: `${fill}%` }}
        />
        {markers.map((marker) => (
          <span
            key={marker.label}
            title={marker.label}
            data-marker={marker.label}
            className="absolute top-0 h-full w-0.5 bg-error"
            style={{ left: `${marker.at}%` }}
          />
        ))}
      </div>
    </div>
  );
}

function Row({ script }: { script: ScriptResource }) {
  const { t } = useTranslation();
  const state =
    script.state === "ready"
      ? t("resources.ready")
      : script.state === "failed"
        ? t("resources.failed")
        : t("resources.refused");
  return (
    <li data-testid={`script-${script.id}`} className="py-2 border-t border-border-light">
      <div className="flex justify-between gap-2">
        <span className="text-body text-text">
          {script.id}
          {script.version ? ` ${script.version}` : ""}
        </span>
        <span
          className={`text-small ${script.state === "ready" ? "text-text-secondary" : "text-error"}`}
        >
          {state}
        </span>
      </div>
      <p className="text-small text-text-tertiary">
        {script.source === "installed" ? t("resources.installed") : t("resources.builtIn")}
        {script.state !== "refused" &&
          ` · ${t("resources.memory", { held: kb(script.held), limit: kb(script.limit) })}`}
        {` · ${t("resources.instances", { count: script.instances })}`}
        {script.state !== "refused" &&
          ` · ${t("resources.calls", {
            count: script.calls,
            average: script.averageUs,
            max: script.maxUs,
          })}`}
      </p>
      {script.reason && <p className="text-small text-error">{script.reason}</p>}
    </li>
  );
}

export function ResourcesCard() {
  const { t } = useTranslation();
  const [resources, setResources] = useState<Resources | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    getResources().then(
      (r) => alive && setResources(r),
      (err: unknown) => alive && setError(formErrorText(err, t)),
    );
    return () => {
      alive = false;
    };
  }, []);

  const { internal, psram, storage, scripts, catalogue } = resources ?? {
    internal: null,
    psram: null,
    storage: null,
    scripts: [] as ScriptResource[],
    catalogue: null,
  };

  return (
    <section className="card p-5">
      <h2 className="text-body font-semibold text-text mb-1">{t("resources.title")}</h2>
      <p className="text-caption text-text-tertiary mb-4">{t("resources.description")}</p>
      {error && <p className="text-small text-error">{error}</p>}
      {psram && (
        <Bar
          testId="psram"
          label={t("resources.psram")}
          fill={percent(psram.total - psram.free, psram.total)}
          markers={[
            {
              at: percent(psram.total - psram.minFree, psram.total),
              label: t("resources.lowest"),
            },
            {
              at: percent(psram.total - psram.reserve, psram.total),
              label: t("resources.reserve"),
            },
          ]}
          summary={t("resources.psramSummary", {
            free: kb(psram.free),
            total: kb(psram.total),
            min: kb(psram.minFree),
            reserve: kb(psram.reserve),
          })}
        />
      )}
      {internal && (
        <Bar
          testId="internal"
          label={t("resources.internal")}
          fill={percent(internal.largestBlock, internal.free)}
          markers={[{ at: percent(internal.minFree, internal.free), label: t("resources.lowest") }]}
          summary={t("resources.internalSummary", {
            free: kb(internal.free),
            min: kb(internal.minFree),
            block: kb(internal.largestBlock),
          })}
        />
      )}
      {storage && (
        <p data-testid="storage" className="text-small text-text-secondary mb-3">
          {storage.total !== null
            ? t("resources.storageOf", { used: kb(storage.used), total: kb(storage.total) })
            : t("resources.storage", { used: kb(storage.used) })}
        </p>
      )}
      {catalogue?.readAt && (
        <p data-testid="catalogue-read" className="text-small text-text-secondary mb-3">
          {t("resources.catalogueRead", { date: new Date(catalogue.readAt).toLocaleString() })}
        </p>
      )}
      {catalogue?.error && (
        <p data-testid="catalogue-error" className="text-small text-error mb-3">
          {t("recipeStore.readError", { reason: catalogue.error })}
        </p>
      )}
      {scripts.length === 0 && resources && (
        <p className="text-small text-text-tertiary">{t("resources.noScripts")}</p>
      )}
      {scripts.length > 0 && (
        <ul>
          {scripts.map((script) => (
            <Row key={script.id} script={script} />
          ))}
        </ul>
      )}
    </section>
  );
}
