import { useEffect, useMemo, useRef, useState } from "react";
import { Eye, Map as MapIcon, Radar, Scale, TriangleAlert } from "lucide-react";
import { Button, Field, bytes, date } from "./ui";
import { call } from "../services/api";
import type {
  Backup,
  WorldDiff,
  WorldDimension,
  WorldMap,
  WorldPreview,
  WorldRegion,
  WorldTrack,
} from "../types";
import type { TaskProps } from "../utils/task";
import { useI18n } from "../i18n";

const dimensionKey = (dimension: WorldDimension) =>
  `${dimension.world}\u0000${dimension.dimension}`;
const regionKey = (region: WorldRegion) => `${region.x},${region.z}`;
const dimensionLabelKey = (label: string) =>
  label === "overworld"
    ? "backups.worldOverworld"
    : label === "the_nether"
      ? "backups.worldNether"
      : label === "the_end"
        ? "backups.worldEnd"
        : null;

export function WorldPreviewPanel({
  id,
  backups,
  run,
  busy,
  running,
}: TaskProps & { id: string; backups: Backup[]; running: boolean }) {
  const { t } = useI18n();
  const [backupId, setBackupId] = useState(backups[0]?.id ?? "");
  const [preview, setPreview] = useState<WorldPreview | null>(null);
  const [selectedDimension, setSelectedDimension] = useState("");
  const [selectedRegion, setSelectedRegion] = useState("");
  const [scale, setScale] = useState(4);
  const [map, setMap] = useState<WorldMap | null>(null);
  const [tracks, setTracks] = useState<WorldTrack[]>([]);
  const [showPlayers, setShowPlayers] = useState(true);
  const [showTracks, setShowTracks] = useState(true);
  const [beforeId, setBeforeId] = useState(backups[1]?.id ?? backups[0]?.id ?? "");
  const [afterId, setAfterId] = useState(backups[0]?.id ?? "");
  const [diff, setDiff] = useState<WorldDiff | null>(null);
  const canvas = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    setBackupId((current) =>
      backups.some((backup) => backup.id === current)
        ? current
        : (backups[0]?.id ?? ""),
    );
    setBeforeId((current) =>
      backups.some((backup) => backup.id === current)
        ? current
        : (backups[1]?.id ?? backups[0]?.id ?? ""),
    );
    setAfterId((current) =>
      backups.some((backup) => backup.id === current)
        ? current
        : (backups[0]?.id ?? ""),
    );
  }, [backups]);

  const dimensions = preview?.dimensions ?? [];
  const activeDimension = useMemo(
    () =>
      dimensions.find((item) => dimensionKey(item) === selectedDimension) ??
      dimensions.find((item) => item.regions.length) ??
      dimensions[0],
    [dimensions, selectedDimension],
  );
  const activeRegion = activeDimension?.regions.find(
    (region) => regionKey(region) === selectedRegion,
  );

  const showLabel = (label: string) => {
    const key = dimensionLabelKey(label);
    return key ? t(key) : label;
  };

  const fetchMap = async (
    snapshot: WorldPreview,
    dimension: WorldDimension,
    region: WorldRegion,
    nextScale: number,
  ) => {
    const result = await call("world_map", {
      id,
      backupId: snapshot.backupId,
      world: dimension.world,
      dimension: dimension.dimension,
      regionX: region.x,
      regionZ: region.z,
      scale: nextScale,
    });
    setMap(result);
    setTracks(result.tracks);
  };

  const inspect = () =>
    void run("worldPreview", async () => {
      const result = await call("world_preview", { id, backupId });
      setPreview(result);
      setDiff(null);
      setMap(null);
      setTracks(result.tracks);
      const dimension =
        result.dimensions.find((item) => item.regions.length > 0) ??
        result.dimensions[0];
      if (!dimension) {
        setSelectedDimension("");
        setSelectedRegion("");
        return;
      }
      setSelectedDimension(dimensionKey(dimension));
      const region = dimension.regions[0];
      setSelectedRegion(region ? regionKey(region) : "");
      if (region) {
        await fetchMap(result, dimension, region, scale);
      }
    });

  const chooseDimension = (value: string) => {
    setSelectedDimension(value);
    const dimension = dimensions.find((item) => dimensionKey(item) === value);
    const region = dimension?.regions[0];
    setSelectedRegion(region ? regionKey(region) : "");
    setMap(null);
    if (preview && dimension && region) {
      void run("worldMap", () =>
        fetchMap(preview, dimension, region, scale),
      );
    }
  };

  const chooseRegion = (value: string) => {
    setSelectedRegion(value);
    const region = activeDimension?.regions.find(
      (item) => regionKey(item) === value,
    );
    setMap(null);
    if (preview && activeDimension && region) {
      void run("worldMap", () =>
        fetchMap(preview, activeDimension, region, scale),
      );
    }
  };

  useEffect(() => {
    if (!map || !canvas.current) return;
    const element = canvas.current;
    element.width = map.width;
    element.height = map.height;
    const context = element.getContext("2d");
    if (!context) return;
    const raw = window.atob(map.pixels);
    const pixels = new Uint8ClampedArray(raw.length);
    for (let index = 0; index < raw.length; index += 1) {
      pixels[index] = raw.charCodeAt(index);
    }
    context.putImageData(new ImageData(pixels, map.width, map.height), 0, 0);
    const inside = (x: number, z: number) =>
      x >= map.originX &&
      x < map.originX + 512 &&
      z >= map.originZ &&
      z < map.originZ + 512;
    if (showPlayers) {
      for (const player of map.players) {
        if (!inside(player.x, player.z)) continue;
        const x = (player.x - map.originX) / map.scale;
        const z = (player.z - map.originZ) / map.scale;
        context.beginPath();
        context.arc(x, z, Math.max(3, 12 / map.scale), 0, Math.PI * 2);
        context.fillStyle = "#f8fafc";
        context.fill();
        context.lineWidth = Math.max(1.5, 3 / map.scale);
        context.strokeStyle = "#d97706";
        context.stroke();
      }
    }
    if (showTracks) {
      for (const track of tracks) {
        const points = track.points.filter(
          (point) =>
            point.dimension === map.label && inside(point.x, point.z),
        );
        if (points.length < 2) continue;
        context.beginPath();
        points.forEach((point, index) => {
          const x = (point.x - map.originX) / map.scale;
          const z = (point.z - map.originZ) / map.scale;
          if (index === 0) context.moveTo(x, z);
          else context.lineTo(x, z);
        });
        context.lineWidth = Math.max(1.5, 3 / map.scale);
        context.strokeStyle = "#2563eb";
        context.stroke();
      }
    }
  }, [map, tracks, showPlayers, showTracks]);

  const sample = () =>
    void run("worldTrackSample", async () => {
      const result = await call("sample_world_tracks", { id });
      setTracks(result);
      if (map) setMap({ ...map, tracks: result });
    });

  const compare = () =>
    void run("worldDiff", async () => {
      const result = await call("world_diff", { id, beforeId, afterId });
      setDiff(result);
    });

  const working =
    busy.has("worldPreview") ||
    busy.has("worldMap") ||
    busy.has("worldDiff") ||
    busy.has("worldTrackSample");

  return (
    <div className="backup-storage world-preview">
      <div className="storage-title">
        <div>
          <h3>
            <MapIcon size={16} />
            {t("backups.worldTitle")}
          </h3>
          <p>{t("backups.worldDesc")}</p>
        </div>
        <span className="tag">{t("backups.worldReadOnly")}</span>
      </div>
      {!backups.length ? (
        <p className="small muted">{t("backups.worldNoBackups")}</p>
      ) : (
        <div className="world-controls">
          <Field label={t("backups.worldSelectBackup")}>
            <select
              value={backupId}
              disabled={working}
              onChange={(event) => setBackupId(event.target.value)}
            >
              {backups.map((backup) => (
                <option key={backup.id} value={backup.id}>
                  {backup.label || date(backup.createdAt)} ·{" "}
                  {bytes(backup.sizeBytes)}
                </option>
              ))}
            </select>
          </Field>
          <Button
            variant="primary"
            disabled={working || !backupId}
            busy={busy.has("worldPreview")}
            onClick={inspect}
          >
            <Eye size={16} />
            {t("backups.worldInspect")}
          </Button>
        </div>
      )}
      {preview && (
        <>
          <div className="storage-stats world-stats">
            <div>
              <small>{t("backups.worldDimensions")}</small>
              <strong>{preview.totals.dimensions}</strong>
            </div>
            <div>
              <small>{t("backups.worldRegions")}</small>
              <strong>{preview.totals.regions}</strong>
            </div>
            <div>
              <small>{t("backups.worldChunks")}</small>
              <strong>{preview.totals.chunks}</strong>
            </div>
            <div>
              <small>{t("backups.worldCorruptChunks")}</small>
              <strong>{preview.totals.corruptChunks}</strong>
            </div>
            <div>
              <small>{t("backups.worldBlockEntities")}</small>
              <strong>{preview.totals.blockEntities}</strong>
            </div>
            <div>
              <small>{t("backups.worldEntities")}</small>
              <strong>{preview.totals.entities}</strong>
            </div>
          </div>
          <p className="small muted">
            {t("backups.worldSnapshotTime", {
              date: date(preview.backupCreatedAt),
              scanned: date(preview.generatedAt),
            })}
          </p>
          {preview.warnings.map((warning) => (
            <p className="warning-text" key={warning}>
              <TriangleAlert size={14} /> {warning}
            </p>
          ))}
          {dimensions.length > 0 && (
            <>
              <div className="world-dimensions">
                {dimensions.map((dimension) => (
                  <button
                    key={dimensionKey(dimension)}
                    type="button"
                    className={
                      activeDimension === dimension ? "active" : undefined
                    }
                    onClick={() => chooseDimension(dimensionKey(dimension))}
                  >
                    <strong>{showLabel(dimension.label)}</strong>
                    <small>
                      {dimension.world} ·{" "}
                      {t("backups.worldRegionCount", {
                        count: dimension.regions.length,
                      })}
                    </small>
                  </button>
                ))}
              </div>
              {activeDimension && (
                <>
                  <div className="world-controls">
                    <Field label={t("backups.worldRegion")}>
                      <select
                        value={selectedRegion}
                        disabled={working || !activeDimension.regions.length}
                        onChange={(event) => chooseRegion(event.target.value)}
                      >
                        {activeDimension.regions.map((region) => (
                          <option key={regionKey(region)} value={regionKey(region)}>
                            r.{region.x}.{region.z} · {region.x * 512},
                            {region.z * 512} · {bytes(region.bytes)}
                          </option>
                        ))}
                      </select>
                    </Field>
                    <Field label={t("backups.worldScale")}>
                      <select
                        value={scale}
                        disabled={working}
                        onChange={(event) => {
                          const next = Number(event.target.value);
                          setScale(next);
                          if (preview && activeDimension && activeRegion) {
                            void run("worldMap", () =>
                              fetchMap(
                                preview,
                                activeDimension,
                                activeRegion,
                                next,
                              ),
                            );
                          }
                        }}
                      >
                        <option value={1}>1 px = 1 block</option>
                        <option value={2}>1 px = 2 blocks</option>
                        <option value={4}>1 px = 4 blocks</option>
                        <option value={8}>1 px = 8 blocks</option>
                      </select>
                    </Field>
                    <label className="check-row">
                      <input
                        type="checkbox"
                        checked={showPlayers}
                        onChange={(event) => setShowPlayers(event.target.checked)}
                      />
                      <span>{t("backups.worldShowPlayers")}</span>
                    </label>
                    <label className="check-row">
                      <input
                        type="checkbox"
                        checked={showTracks}
                        onChange={(event) => setShowTracks(event.target.checked)}
                      />
                      <span>{t("backups.worldShowTracks")}</span>
                    </label>
                  </div>
                  {activeDimension.regions.length === 0 ? (
                    <p className="small muted">
                      {t("backups.worldNoRegions")}
                    </p>
                  ) : (
                    <div className="world-map-wrap">
                      <canvas ref={canvas} className="world-map" />
                      {working && (
                        <div className="world-map-loading">
                          {t("backups.worldRendering")}
                        </div>
                      )}
                    </div>
                  )}
                  {map && (
                    <p className="small muted">
                      {t("backups.worldMapMeta", {
                        chunks: map.chunks,
                        empty: map.emptyChunks,
                        corrupt: map.corruptChunks,
                      })}
                    </p>
                  )}
                  {map?.warnings.map((warning) => (
                    <p className="warning-text" key={warning}>
                      <TriangleAlert size={14} /> {warning}
                    </p>
                  ))}
                  <div className="world-dimension-stats">
                    <span>
                      {t("backups.worldChunks")}: {activeDimension.chunks}
                    </span>
                    <span>
                      {t("backups.worldCorruptChunks")}:{" "}
                      {activeDimension.corruptChunks}
                    </span>
                    <span>
                      {t("backups.worldBlockEntities")}:{" "}
                      {activeDimension.blockEntities}
                    </span>
                    <span>
                      {t("backups.worldEntities")}:{" "}
                      {activeDimension.entities}
                    </span>
                    {activeDimension.truncated && (
                      <span className="warning-text">
                        {t("backups.worldTruncated")}
                      </span>
                    )}
                  </div>
                  {(activeDimension.blockEntityTypes.length > 0 ||
                    activeDimension.entityTypes.length > 0) && (
                    <div className="world-types">
                      {activeDimension.blockEntityTypes.length > 0 && (
                        <div>
                          <small>{t("backups.worldBlockEntityTypes")}</small>
                          <div className="tags">
                            {activeDimension.blockEntityTypes.map((entry) => (
                              <span className="tag" key={entry.id}>
                                {entry.id} × {entry.count}
                              </span>
                            ))}
                          </div>
                        </div>
                      )}
                      {activeDimension.entityTypes.length > 0 && (
                        <div>
                          <small>{t("backups.worldEntityTypes")}</small>
                          <div className="tags">
                            {activeDimension.entityTypes.map((entry) => (
                              <span className="tag" key={entry.id}>
                                {entry.id} × {entry.count}
                              </span>
                            ))}
                          </div>
                        </div>
                      )}
                    </div>
                  )}
                </>
              )}
            </>
          )}
          <div className="world-columns">
            <div className="world-list">
              <div className="remote-history-head">
                <strong>{t("backups.worldPlayers")}</strong>
                <small>{preview.players.length}</small>
              </div>
              {preview.players.length ? (
                preview.players.map((player) => (
                  <div className="world-player" key={player.uuid}>
                    <strong>{player.name}</strong>
                    <span>
                      {showLabel(player.dimension)} · {player.x.toFixed(1)},{" "}
                      {player.y.toFixed(1)}, {player.z.toFixed(1)}
                    </span>
                    <small>
                      {t("backups.worldPlayerHealth", {
                        health: player.health.toFixed(1),
                        food: player.food,
                        xp: player.xp,
                      })}
                    </small>
                  </div>
                ))
              ) : (
                <p className="small muted">{t("backups.worldNoPlayers")}</p>
              )}
              <p className="small muted">{t("backups.worldPlayersNote")}</p>
            </div>
            <div className="world-list">
              <div className="remote-history-head">
                <strong>{t("backups.worldTracks")}</strong>
                <Button
                  disabled={working || !running}
                  busy={busy.has("worldTrackSample")}
                  title={
                    running
                      ? undefined
                      : t("backups.worldTracksNeedServer")
                  }
                  onClick={sample}
                >
                  <Radar size={15} />
                  {t("backups.worldSample")}
                </Button>
              </div>
              {tracks.length ? (
                tracks.map((track) => (
                  <div className="world-player" key={track.player}>
                    <strong>{track.player}</strong>
                    <span>
                      {t("backups.worldTrackPoints", {
                        count: track.points.length,
                      })}
                    </span>
                    <small>
                      {track.points[track.points.length - 1]
                        ? `${showLabel(track.points[track.points.length - 1].dimension)} · ${date(track.points[track.points.length - 1].at)}`
                        : ""}
                    </small>
                  </div>
                ))
              ) : (
                <p className="small muted">{t("backups.worldNoTracks")}</p>
              )}
              <p className="small muted">{t("backups.worldTracksNote")}</p>
            </div>
          </div>
          <div className="world-list">
            <div className="remote-history-head">
              <strong>{t("backups.worldCorruptions")}</strong>
              <small>
                {preview.corruptions.length}
                {preview.omittedCorruptions
                  ? ` + ${preview.omittedCorruptions}`
                  : ""}
              </small>
            </div>
            {preview.corruptions.length ? (
              <div className="world-corruptions">
                {preview.corruptions.map((item, index) => (
                  <div
                    key={`${item.regionX}.${item.regionZ}.${item.chunkX}.${item.chunkZ}.${index}`}
                  >
                    <span className="tag">
                      r.{item.regionX}.{item.regionZ}
                    </span>
                    <span>
                      {item.chunkX === null
                        ? t("backups.worldRegionFile")
                        : `chunk ${item.chunkX},${item.chunkZ}`}
                    </span>
                    <small>{item.reason}</small>
                  </div>
                ))}
              </div>
            ) : (
              <p className="small muted">{t("backups.worldNoCorruptions")}</p>
            )}
            {preview.omittedCorruptions > 0 && (
              <p className="small muted">
                {t("backups.worldCorruptionsMore", {
                  count: preview.omittedCorruptions,
                })}
              </p>
            )}
          </div>
        </>
      )}
      {backups.length > 1 && (
        <div className="world-diff">
          <div className="storage-title">
            <div>
              <h3>
                <Scale size={16} />
                {t("backups.worldDiffTitle")}
              </h3>
              <p>{t("backups.worldDiffDesc")}</p>
            </div>
          </div>
          <div className="world-controls">
            <Field label={t("backups.worldBefore")}>
              <select
                value={beforeId}
                disabled={working}
                onChange={(event) => setBeforeId(event.target.value)}
              >
                {backups.map((backup) => (
                  <option key={backup.id} value={backup.id}>
                    {backup.label || date(backup.createdAt)}
                  </option>
                ))}
              </select>
            </Field>
            <Field label={t("backups.worldAfter")}>
              <select
                value={afterId}
                disabled={working}
                onChange={(event) => setAfterId(event.target.value)}
              >
                {backups.map((backup) => (
                  <option key={backup.id} value={backup.id}>
                    {backup.label || date(backup.createdAt)}
                  </option>
                ))}
              </select>
            </Field>
            <Button
              disabled={working || !beforeId || !afterId || beforeId === afterId}
              busy={busy.has("worldDiff")}
              onClick={compare}
            >
              {t("backups.worldCompare")}
            </Button>
          </div>
          {diff && (
            <>
              <div className="world-diff-stats">
                <span className="tag">
                  {t("backups.worldAdded")}: {diff.added}
                </span>
                <span className="tag">
                  {t("backups.worldRemoved")}: {diff.removed}
                </span>
                <span className="tag">
                  {t("backups.worldChanged")}: {diff.changed}
                </span>
                <span className="tag">
                  {t("backups.worldUnchanged")}: {diff.unchanged}
                </span>
              </div>
              {diff.changes.length ? (
                <div className="backup-table world-diff-table">
                  <div className="backup-head">
                    <span>{t("backups.worldRegion")}</span>
                    <span>{t("backups.worldStatus")}</span>
                    <span>{t("backups.worldSizeChange")}</span>
                  </div>
                  {diff.changes.map((change) => (
                    <div
                      className="backup-row"
                      key={`${change.world}.${change.dimension}.${change.regionX}.${change.regionZ}`}
                    >
                      <span>
                        <strong>
                          r.{change.regionX}.{change.regionZ}
                        </strong>
                        <small>
                          {change.world} · {showLabel(change.label)}
                        </small>
                      </span>
                      <span className="tag">{change.status}</span>
                      <span>
                        {bytes(change.beforeBytes)} →{" "}
                        {bytes(change.afterBytes)}
                      </span>
                    </div>
                  ))}
                </div>
              ) : (
                <p className="small muted">{t("backups.worldDiffEmpty")}</p>
              )}
              {diff.truncated && (
                <p className="small muted">{t("backups.worldDiffTruncated")}</p>
              )}
            </>
          )}
        </div>
      )}
    </div>
  );
}
