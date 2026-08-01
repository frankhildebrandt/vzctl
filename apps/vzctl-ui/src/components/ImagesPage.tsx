import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import {
  bakeImage,
  catalogAliasOptions,
  imageKeys,
  imageStateLabel,
  listImages,
  pullImage,
  sealImage,
  type ImageListItem,
} from "@/lib/images";

export function ImagesPage() {
  const queryClient = useQueryClient();
  const [actionMsg, setActionMsg] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busyAlias, setBusyAlias] = useState<string | null>(null);
  const [pullAlias, setPullAlias] = useState("ubuntu-latest");
  const [sealTarget, setSealTarget] = useState("");

  const listQuery = useQuery({
    queryKey: imageKeys.list(),
    queryFn: listImages,
  });

  const catalogOptions = useMemo(
    () => catalogAliasOptions(listQuery.data?.catalog ?? []),
    [listQuery.data?.catalog],
  );

  const invalidate = () =>
    void queryClient.invalidateQueries({ queryKey: imageKeys.all });

  const pullMutation = useMutation({
    mutationFn: (alias: string) => pullImage(alias),
    onMutate: (alias) => {
      setError(null);
      setActionMsg(null);
      setBusyAlias(alias);
    },
    onSuccess: (_data, alias) => {
      setActionMsg(`Pull ok: ${alias}`);
      invalidate();
    },
    onError: (err) => setError(String(err)),
    onSettled: () => setBusyAlias(null),
  });

  const bakeMutation = useMutation({
    mutationFn: (alias: string) => bakeImage(alias),
    onMutate: (alias) => {
      setError(null);
      setActionMsg(null);
      setBusyAlias(alias);
    },
    onSuccess: (_data, alias) => {
      setActionMsg(`Bake ok: ${alias}`);
      invalidate();
    },
    onError: (err) => setError(String(err)),
    onSettled: () => setBusyAlias(null),
  });

  const sealMutation = useMutation({
    mutationFn: (target: string) => sealImage(target),
    onMutate: (target) => {
      setError(null);
      setActionMsg(null);
      setBusyAlias(target);
    },
    onSuccess: (_data, target) => {
      setActionMsg(`Seal ok: ${target}`);
      invalidate();
    },
    onError: (err) => setError(String(err)),
    onSettled: () => setBusyAlias(null),
  });

  const images = listQuery.data?.images ?? [];
  const busy =
    pullMutation.isPending ||
    bakeMutation.isPending ||
    sealMutation.isPending ||
    listQuery.isFetching;

  return (
    <section>
      <header className="detail-heading" style={{ marginBottom: "1rem" }}>
        <h2 className="section-title">Images</h2>
        <p className="muted">
          Image-Cache wie <code>vzctl image list|pull|bake|seal</code> — Lifecycle{" "}
          <code>pull → bake → seal</code>.
        </p>
      </header>

      <div className="card summary-card">
        <div className="summary-row">
          <span className="badge ok">{images.length} lokal</span>
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
            Aktualisieren
          </button>
        </div>
        {listQuery.isError ? (
          <p className="tile-error">{String(listQuery.error)}</p>
        ) : null}
        {error ? <p className="tile-error">{error}</p> : null}
        {actionMsg ? <p className="muted">{actionMsg}</p> : null}
        {busyAlias ? (
          <p className="muted">
            Läuft: <code>{busyAlias}</code> (kann Minuten dauern)…
          </p>
        ) : null}
      </div>

      <div className="card">
        <h3 className="group-title">Pull</h3>
        <p className="muted">
          Alias aus dem Katalog laden und als Raw normalisieren.
        </p>
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
            Alias
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
            Pull
          </button>
        </form>
      </div>

      <div className="card">
        <h3 className="group-title">Seal (Name / Pfad)</h3>
        <p className="muted">
          Freies Input wie CLI <code>image seal &lt;name|path&gt;</code>.
        </p>
        <form
          className="form-grid"
          style={{ alignItems: "end" }}
          onSubmit={(event) => {
            event.preventDefault();
            const target = sealTarget.trim();
            if (target) sealMutation.mutate(target);
          }}
        >
          <label>
            Name oder Pfad
            <input
              value={sealTarget}
              disabled={busy}
              onChange={(e) => setSealTarget(e.target.value)}
              placeholder="ubuntu-latest oder /path/to/image.raw"
              list="image-seal-aliases"
            />
            <datalist id="image-seal-aliases">
              {images.map((image) => (
                <option key={image.alias} value={image.alias} />
              ))}
            </datalist>
          </label>
          <button type="submit" disabled={busy || !sealTarget.trim()}>
            Seal
          </button>
        </form>
      </div>

      {listQuery.isLoading ? (
        <p className="muted">Lade Image-Cache…</p>
      ) : images.length === 0 ? (
        <div className="card">
          <h2>Kein lokaler Cache</h2>
          <p className="muted">
            Noch keine Aliase gepullt. Oben einen Katalog-Alias pullen.
          </p>
        </div>
      ) : (
        <div className="card" style={{ padding: 0, overflow: "auto" }}>
          <table className="vm-table">
            <thead>
              <tr>
                <th>Alias</th>
                <th>State</th>
                <th>Distribution</th>
                <th>Release</th>
                <th>Agent</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {images.map((image) => (
                <ImageRow
                  key={image.alias}
                  image={image}
                  busy={busy}
                  rowBusy={busyAlias === image.alias}
                  onBake={() => bakeMutation.mutate(image.alias)}
                  onSeal={() => sealMutation.mutate(image.alias)}
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
  rowBusy,
  onBake,
  onSeal,
}: {
  image: ImageListItem;
  busy: boolean;
  rowBusy: boolean;
  onBake: () => void;
  onSeal: () => void;
}) {
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
      <td>{image.distribution || "—"}</td>
      <td>{image.release || "—"}</td>
      <td className="muted">{image.agent_version ?? "—"}</td>
      <td>
        <div className="row" style={{ gap: "0.35rem", justifyContent: "flex-end" }}>
          <button
            type="button"
            className="secondary"
            disabled={busy || image.sealed}
            title={
              image.sealed
                ? "Bereits sealed — Bake nicht möglich"
                : "Guest-Agent einbacken"
            }
            onClick={onBake}
          >
            {rowBusy ? "…" : "Bake"}
          </button>
          <button
            type="button"
            className="secondary"
            disabled={busy}
            title="Clone-safe versiegeln"
            onClick={onSeal}
          >
            Seal
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
