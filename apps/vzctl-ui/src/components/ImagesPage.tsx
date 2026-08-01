import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo, useRef, useState } from "react";
import { ConsoleLog, type ConsoleLine } from "@/components/ApplyProgress";
import { useT } from "@/lib/i18n";
import { localeToBcp47 } from "@/lib/i18n/detect";
import {
  bakeImage,
  catalogAliasOptions,
  DEFAULT_IMAGE_TAG,
  imageKeys,
  imageStateLabel,
  listImages,
  pullImage,
  sealImage,
  validImageTag,
  type ImageListItem,
  type JobResponse,
} from "@/lib/images";
import { useSettingsStore } from "@/store/settingsStore";

export function ImagesPage() {
  const t = useT();
  const queryClient = useQueryClient();
  const [actionMsg, setActionMsg] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busyAlias, setBusyAlias] = useState<string | null>(null);
  const [pullAlias, setPullAlias] = useState("ubuntu-latest");
  const [imageTag, setImageTag] = useState(DEFAULT_IMAGE_TAG);
  const [sealTarget, setSealTarget] = useState("");
  const [jobLines, setJobLines] = useState<ConsoleLine[]>([]);
  const logCursor = useRef(0);
  const nextLineId = useRef(1);

  const listQuery = useQuery({
    queryKey: imageKeys.list(),
    queryFn: listImages,
  });

  const catalogOptions = useMemo(
    () => catalogAliasOptions(listQuery.data?.catalog ?? []),
    [listQuery.data?.catalog],
  );

  const tag = imageTag.trim();
  const tagOk = validImageTag(tag);

  const invalidate = () =>
    void queryClient.invalidateQueries({ queryKey: imageKeys.all });

  const resetJobLog = () => {
    logCursor.current = 0;
    nextLineId.current = 1;
    setJobLines([]);
  };

  const onJobUpdate = (job: JobResponse) => {
    const log = job.log ?? [];
    if (log.length < logCursor.current) {
      logCursor.current = 0;
      nextLineId.current = 1;
      setJobLines([]);
    }
    if (log.length <= logCursor.current) return;
    const locale = useSettingsStore.getState().locale;
    const ts = new Date().toLocaleTimeString(localeToBcp47(locale), {
      hour12: false,
    });
    const fresh = log.slice(logCursor.current).map((text) => {
      const id = nextLineId.current;
      nextLineId.current += 1;
      return { id, ts, level: "info" as const, text };
    });
    logCursor.current = log.length;
    setJobLines((prev) => [...prev, ...fresh]);
  };

  const jobOpts = { onUpdate: onJobUpdate };

  const pullMutation = useMutation({
    mutationFn: (alias: string) => pullImage(alias, jobOpts),
    onMutate: (alias) => {
      setError(null);
      setActionMsg(null);
      setBusyAlias(alias);
      resetJobLog();
    },
    onSuccess: (_data, alias) => {
      setActionMsg(t("images.pullOk", { alias }));
      invalidate();
    },
    onError: (err) => setError(String(err)),
    onSettled: () => setBusyAlias(null),
  });

  const bakeMutation = useMutation({
    mutationFn: ({ alias, tag }: { alias: string; tag: string }) =>
      bakeImage(alias, tag, jobOpts),
    onMutate: ({ alias }) => {
      setError(null);
      setActionMsg(null);
      setBusyAlias(alias);
      resetJobLog();
    },
    onSuccess: (_data, { alias, tag }) => {
      setActionMsg(t("images.bakeOk", { alias, tag }));
      invalidate();
    },
    onError: (err) => setError(String(err)),
    onSettled: () => setBusyAlias(null),
  });

  const sealMutation = useMutation({
    mutationFn: ({ target, tag }: { target: string; tag: string }) =>
      sealImage(target, tag, jobOpts),
    onMutate: ({ target }) => {
      setError(null);
      setActionMsg(null);
      setBusyAlias(target);
      resetJobLog();
    },
    onSuccess: (_data, { target, tag }) => {
      setActionMsg(t("images.sealOk", { target, tag }));
      invalidate();
    },
    onError: (err) => setError(String(err)),
    onSettled: () => setBusyAlias(null),
  });

  const images = listQuery.data?.images ?? [];
  const jobPending =
    pullMutation.isPending || bakeMutation.isPending || sealMutation.isPending;
  const busy = jobPending || listQuery.isFetching;
  const showJobLog = jobPending || jobLines.length > 0;

  return (
    <section>
      <header className="detail-heading" style={{ marginBottom: "1rem" }}>
        <h2 className="section-title">{t("images.title")}</h2>
        <p className="muted">
          {t("images.subtitle")}
        </p>
      </header>

      <div className="card summary-card">
        <div className="summary-row">
          <span className="badge ok">{t("images.localCount", { n: images.length })}</span>
          {listQuery.data?.imagesDir ? (
            <span className="muted" title={listQuery.data.imagesDir}>
              {listQuery.data.imagesDir}
            </span>
          ) : null}
          <button
            type="button"
            className="secondary"
            disabled={busy}
            onClick={() => void listQuery.refetch()}
          >
            {t("images.refresh")}
          </button>
        </div>
        {listQuery.isError ? (
          <p className="tile-error">{String(listQuery.error)}</p>
        ) : null}
        {error ? <p className="tile-error">{error}</p> : null}
        {actionMsg ? <p className="muted">{actionMsg}</p> : null}
        {busyAlias ? (
          <p className="muted">
            {t("images.busy", { alias: busyAlias })}
          </p>
        ) : null}
      </div>

      {showJobLog ? (
        <div style={{ marginBottom: "1rem" }}>
          <h3 className="group-title">{t("images.jobLog.title")}</h3>
          <ConsoleLog lines={jobLines} visible />
        </div>
      ) : null}

      <div className="card">
        <h3 className="group-title">{t("images.pullTitle")}</h3>
        <p className="muted">{t("images.pullHint")}</p>
        <form
          className="form-grid"
          style={{ alignItems: "end" }}
          onSubmit={(event) => {
            event.preventDefault();
            const alias = pullAlias.trim();
            if (alias) pullMutation.mutate(alias);
          }}
        >
          <label>
            {t("images.pullAlias")}
            <select
              value={pullAlias}
              disabled={busy}
              onChange={(e) => setPullAlias(e.target.value)}
            >
              {catalogOptions.map((alias) => (
                <option key={alias} value={alias}>
                  {alias}
                </option>
              ))}
            </select>
          </label>
          <button type="submit" disabled={busy || !pullAlias.trim()}>
            {t("images.pull")}
          </button>
        </form>
      </div>

      <div className="card">
        <h3 className="group-title">{t("images.tagTitle")}</h3>
        <p className="muted">{t("images.tagHint")}</p>
        <label>
          {t("images.tagField")}
          <input
            value={imageTag}
            disabled={busy}
            onChange={(e) => setImageTag(e.target.value)}
            placeholder={DEFAULT_IMAGE_TAG}
            spellCheck={false}
            autoComplete="off"
          />
        </label>
        {!tagOk && imageTag.trim() !== "" ? (
          <p className="form-error">{t("images.tagInvalid")}</p>
        ) : null}
      </div>

      <div className="card">
        <h3 className="group-title">{t("images.sealTitle")}</h3>
        <p className="muted">{t("images.sealHint")}</p>
        <form
          className="form-grid"
          style={{ alignItems: "end" }}
          onSubmit={(event) => {
            event.preventDefault();
            const target = sealTarget.trim();
            if (target && tagOk) sealMutation.mutate({ target, tag });
          }}
        >
          <label>
            {t("images.sealField")}
            <input
              value={sealTarget}
              disabled={busy}
              onChange={(e) => setSealTarget(e.target.value)}
              placeholder={t("images.sealPlaceholder")}
              list="image-seal-aliases"
            />
            <datalist id="image-seal-aliases">
              {images.map((image) => (
                <option key={image.alias} value={image.alias} />
              ))}
            </datalist>
          </label>
          <button type="submit" disabled={busy || !sealTarget.trim() || !tagOk}>
            {t("images.seal")}
          </button>
        </form>
      </div>

      {listQuery.isLoading ? (
        <p className="muted">{t("images.loading")}</p>
      ) : images.length === 0 ? (
        <div className="card">
          <h2>{t("images.emptyTitle")}</h2>
          <p className="muted">{t("images.emptyHint")}</p>
        </div>
      ) : (
        <div className="card" style={{ padding: 0, overflow: "auto" }}>
          <table className="vm-table">
            <thead>
              <tr>
                <th>{t("images.col.alias")}</th>
                <th>{t("images.col.state")}</th>
                <th>{t("images.col.distribution")}</th>
                <th>{t("images.col.release")}</th>
                <th>{t("images.col.agent")}</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {images.map((image) => (
                <ImageRow
                  key={image.alias}
                  image={image}
                  busy={busy}
                  tagOk={tagOk}
                  rowBusy={busyAlias === image.alias}
                  onBake={() =>
                    tagOk && bakeMutation.mutate({ alias: image.alias, tag })
                  }
                  onSeal={() =>
                    tagOk && sealMutation.mutate({ target: image.alias, tag })
                  }
                />
              ))}
            </tbody>
          </table>
        </div>
      )}
    </section>
  );
}

function ImageRow({
  image,
  busy,
  tagOk,
  rowBusy,
  onBake,
  onSeal,
}: {
  image: ImageListItem;
  busy: boolean;
  tagOk: boolean;
  rowBusy: boolean;
  onBake: () => void;
  onSeal: () => void;
}) {
  const t = useT();
  const state = imageStateLabel(image);
  return (
    <tr>
      <td>
        <span className="project-name">{image.alias}</span>
        {image.canonical_alias !== image.alias ? (
          <div className="muted" style={{ fontSize: "0.85rem" }}>
            → {image.canonical_alias}
          </div>
        ) : null}
        <div className="muted" style={{ fontSize: "0.8rem" }} title={image.path}>
          {shortPath(image.path)}
        </div>
      </td>
      <td>
        <span
          className={
            state === "sealed" ? "badge ok" : state === "baked" ? "badge warn" : "badge"
          }
        >
          {state}
        </span>
      </td>
      <td>{image.distribution || t("common.emDash")}</td>
      <td>{image.release || t("common.emDash")}</td>
      <td className="muted">{image.agent_version ?? t("common.emDash")}</td>
      <td>
        <div className="row" style={{ gap: "0.35rem", justifyContent: "flex-end" }}>
          <button
            type="button"
            className="secondary"
            disabled={busy || image.sealed || !tagOk}
            title={
              image.sealed
                ? t("images.bakeSealed")
                : !tagOk
                  ? t("images.tagInvalid")
                  : t("images.bakeHint")
            }
            onClick={onBake}
          >
            {rowBusy ? t("common.ellipsis") : t("images.bake")}
          </button>
          <button
            type="button"
            className="secondary"
            disabled={busy || !tagOk}
            title={!tagOk ? t("images.tagInvalid") : t("images.sealHintBtn")}
            onClick={onSeal}
          >
            {t("images.seal")}
          </button>
        </div>
      </td>
    </tr>
  );
}

function shortPath(path: string): string {
  if (path.length <= 48) return path;
  return `…${path.slice(-45)}`;
}
