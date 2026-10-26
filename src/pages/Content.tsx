import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  ArrowUpCircle,
  Boxes,
  Download,
  FileArchive,
  FolderOpen,
  Package,
  Pin,
  Puzzle,
  RefreshCw,
  Search,
  Trash2,
  Undo2,
} from "lucide-react";
import { Button, Field, Modal, bytes } from "../components/ui";
import { DependencyCheck } from "../components/DependencyCheck";
import { call } from "../services/api";
import { useI18n, type MessageKey } from "../i18n";
import type {
  ContentEntry,
  ContentListing,
  ContentProgress,
  ContentRollback,
  ContentUpdateEntry,
  ContentUpdateReport,
  ExportedPack,
  ModpackInfo,
  ModpackReport,
  ModpackUpdatePreview,
  ProjectHit,
  ProjectVersion,
  ServerView,
} from "../types";
import { idle } from "../types";
import type { TaskProps } from "../utils/task";

type Mode = "content" | "modpack";
type Pending = { title: string; body: string; action: () => Promise<void> };

const compact = (n: number) =>
  n >= 1_000_000
    ? `${(n / 1_000_000).toFixed(1)}M`
    : n >= 1_000
      ? `${(n / 1_000).toFixed(1)}k`
      : String(n);

export function Content({
  server,
  run,
  busy,
}: TaskProps & { server: ServerView }) {
  const { t } = useI18n();
  const plugins =
    server.serverType === "paper" || server.serverType === "purpur";
  const kind = plugins ? "plugin" : "mod";
  const loader = plugins ? "paper" : "fabric";
  const modes: Mode[] = server.serverType === "vanilla"
    ? ["modpack"]
    : plugins
      ? ["content"]
      : ["content", "modpack"];
  const [mode, setMode] = useState<Mode>(modes[0]);
  const [listing, setListing] = useState<ContentListing | null>(null);
  const [query, setQuery] = useState("");
  const [page, setPage] = useState<{
    hits: ProjectHit[];
    total: number;
    offset: number;
  } | null>(null);
  const [active, setActive] = useState<ProjectHit | null>(null);
  const [versions, setVersions] = useState<ProjectVersion[] | null>(null);
  const [progress, setProgress] = useState<ContentProgress | null>(null);
  const [report, setReport] = useState<ModpackReport | null>(null);
  const [pack, setPack] = useState<ModpackInfo | null>(null);
  const [packPreview, setPackPreview] = useState<ModpackUpdatePreview | null>(
    null,
  );
  const [packMessage, setPackMessage] = useState<string | null>(null);
  const [packRollbackOpen, setPackRollbackOpen] = useState(false);
  const [pending, setPending] = useState<Pending | null>(null);
  const [updates, setUpdates] = useState<ContentUpdateReport | null>(null);
  const [updateTarget, setUpdateTarget] = useState<ContentUpdateEntry | null>(
    null,
  );
  const [rollbackTarget, setRollbackTarget] = useState<{
    entry: ContentEntry;
    version: string;
  } | null>(null);
  const [updateMessage, setUpdateMessage] = useState<string | null>(null);
  const [exportOpen, setExportOpen] = useState(false);
  const [exportForm, setExportForm] = useState({
    path: "",
    name: "",
    version: "1.0.0",
  });
  const [exportResult, setExportResult] = useState<ExportedPack | null>(null);
  const refreshKey = `content-list-${server.id}`;
  const searchKey = `content-search-${server.id}`;
  const installKey = `content-install-${server.id}`;
  const checkKey = `content-check-${server.id}`;
  const updateKey = `content-update-${server.id}`;
  const rollbackKey = `content-rollback-${server.id}`;
  const pinKey = `content-pin-${server.id}`;
  const packCheckKey = `modpack-check-${server.id}`;
  const packUpdateKey = `modpack-update-${server.id}`;
  const packRollbackKey = `modpack-rollback-${server.id}`;
  const exportPickKey = `modpack-export-pick-${server.id}`;
  const exportKey = `modpack-export-${server.id}`;
  const loadListing = async () => {
    const [content, installedPack] = await Promise.all([
      call("content_list", { id: server.id }),
      call("modpack_info", { id: server.id }),
    ]);
    setListing(content);
    setPack(installedPack);
  };
  useEffect(() => {
    void run(refreshKey, loadListing);
    setListing(null);
    setPage(null);
    setActive(null);
    setVersions(null);
    setProgress(null);
    setReport(null);
    setPack(null);
    setPackPreview(null);
    setPackMessage(null);
    setPackRollbackOpen(false);
    setUpdates(null);
    setUpdateTarget(null);
    setRollbackTarget(null);
    setUpdateMessage(null);
    setExportOpen(false);
    setExportResult(null);
    setMode(modes[0]);
  }, [server.id]);
  useEffect(() => {
    const subscription = listen<ContentProgress>(
      "content-progress",
      (event) => {
        if (event.payload.id === server.id) setProgress(event.payload);
      },
    );
    return () => {
      void subscription.then((stop) => stop());
    };
  }, [server.id]);
  const search = (offset = 0) =>
    void run(searchKey, async () => {
      const result = await call("modrinth_search", {
        query,
        projectType: mode === "modpack" ? "modpack" : kind,
        gameVersion: server.minecraftVersion,
        loader: mode === "modpack" ? undefined : loader,
        offset,
      });
      setPage((previous) =>
        previous && offset > 0
          ? {
              hits: [...previous.hits, ...result.hits],
              total: result.total,
              offset: result.offset,
            }
          : result,
      );
    });
  const browse = (hit: ProjectHit) => {
    setActive(hit);
    setVersions(null);
    void run(`content-versions-${hit.id}`, async () => {
      setVersions(
        await call("modrinth_versions", {
          projectId: hit.id,
          gameVersion: server.minecraftVersion,
          loader: mode === "modpack" ? undefined : loader,
        }),
      );
    });
  };
  const install = (version: ProjectVersion) => {
    if (!active) return;
    void run(installKey, async () => {
      setProgress(null);
      setReport(null);
      if (mode === "modpack") {
        setReport(
          await call("modpack_install", {
            id: server.id,
            projectId: active.id,
            versionId: version.id,
          }),
        );
      } else {
        await call("modrinth_install", {
          id: server.id,
          projectId: active.id,
          versionId: version.id,
          kind,
        });
      }
      setActive(null);
      setVersions(null);
      await loadListing();
    });
  };
  const removeEntry = (entry: ContentEntry) =>
    setPending({
      title: t("content.removeTitle", { name: entry.title }),
      body: t("content.removeBody"),
      action: async () => {
        await call("content_remove", {
          id: server.id,
          projectId: entry.projectId,
        });
        await loadListing();
      },
    });
  const removeFile = (name: string) =>
    setPending({
      title: t("content.removeTitle", { name }),
      body: t("content.removeFileBody", { name }),
      action: async () => {
        await call("content_remove_file", {
          id: server.id,
          kind,
          name,
        });
        await loadListing();
      },
    });
  const checkUpdates = () => {
    setUpdateMessage(null);
    void run(checkKey, async () => {
      setUpdates(await call("content_check_updates", { id: server.id }));
    });
  };
  const applyUpdate = (entry: ContentUpdateEntry) => {
    if (!entry.latestVersionId) return;
    void run(updateKey, async () => {
      setProgress(null);
      const outcome = await call("content_update", {
        id: server.id,
        projectId: entry.projectId,
        versionId: entry.latestVersionId!,
        confirmed: true,
      });
      setUpdateTarget(null);
      setUpdates(null);
      setUpdateMessage(
        t("content.updateDone", {
          name: outcome.title,
          from: outcome.fromVersion,
          to: outcome.toVersion,
        }),
      );
      await loadListing();
    });
  };
  const rollbackUpdate = () => {
    if (!rollbackTarget) return;
    void run(rollbackKey, async () => {
      const outcome = await call("content_rollback", {
        id: server.id,
        projectId: rollbackTarget.entry.projectId,
        confirmed: true,
      });
      setRollbackTarget(null);
      setUpdates(null);
      setUpdateMessage(
        t("content.rollbackDone", {
          name: outcome.title,
          version: outcome.version,
        }),
      );
      await loadListing();
    });
  };
  const togglePin = (entry: ContentEntry, pinned: boolean) =>
    void run(pinKey, async () => {
      await call("content_set_pinned", {
        id: server.id,
        projectId: entry.projectId,
        pinned,
      });
      setUpdates(null);
      await loadListing();
    });
  const checkPackUpdate = () => {
    setPackMessage(null);
    void run(packCheckKey, async () => {
      const preview = await call("modpack_update_check", { id: server.id });
      if (preview) setPackPreview(preview);
      else setPackMessage(t("content.packUpToDate"));
    });
  };
  const openExport = () => {
    setExportResult(null);
    setExportForm({
      path: "",
      name: server.name,
      version: "1.0.0",
    });
    setExportOpen(true);
  };
  const pickExportPath = () =>
    void run(exportPickKey, async () => {
      const path = await call("modpack_pick_export", {
        name: exportForm.name.trim() || server.name,
      });
      if (path) {
        setExportForm((previous) => ({ ...previous, path }));
      }
    });
  const exportPack = () => {
    if (!exportForm.path.trim()) return;
    void run(exportKey, async () => {
      setExportResult(null);
      const result = await call("modpack_export", {
        id: server.id,
        input: {
          path: exportForm.path.trim(),
          name: exportForm.name.trim() || undefined,
          version: exportForm.version.trim() || undefined,
        },
      });
      setExportResult(result);
    });
  };
  const applyPackUpdate = () => {
    if (!packPreview) return;
    void run(packUpdateKey, async () => {
      setProgress(null);
      const outcome = await call("modpack_update", {
        id: server.id,
        confirmed: true,
      });
      setPackPreview(null);
      setPackMessage(
        t("content.packUpdateDone", {
          from: outcome.fromVersion,
          to: outcome.toVersion,
        }),
      );
      await loadListing();
    });
  };
  const rollbackPack = () => {
    void run(packRollbackKey, async () => {
      const outcome = await call("modpack_rollback", {
        id: server.id,
        confirmed: true,
      });
      setPackRollbackOpen(false);
      setPackMessage(
        t("content.packRollbackDone", { version: outcome.restoredVersion }),
      );
      await loadListing();
    });
  };
  const warningText = (code: string) =>
    t(`content.warning.${code}` as MessageKey);
  const impactText = (dependency: {
    title: string | null;
    projectId: string;
    status: string;
  }) => {
    const name = dependency.title ?? dependency.projectId;
    return t(`content.impact.${dependency.status}` as MessageKey, { name });
  };
  const rollbackFor = (projectId: string): ContentRollback | undefined =>
    listing?.rollbacks.find((rollback) => rollback.projectId === projectId);
  const isPinned = (projectId: string) =>
    Boolean(listing?.pinned.includes(projectId));
  const progressText = (value: ContentProgress) => {
    const vars = {
      file: value.file,
      done: value.completed,
      total: value.total,
    };
    if (value.stage === "download") return t("content.progress.download", vars);
    if (value.stage === "overrides")
      return t("content.progress.overrides", vars);
    if (value.stage === "reuse")
      return t("content.progress.reuse", vars);
    if (value.stage === "verify") return t("content.progress.verify");
    if (value.stage === "done") return t("content.progress.done");
    return value.stage;
  };
  const installModal = active && (
    <Modal
      title={t("content.versionsTitle", { name: active.title })}
      onClose={() => setActive(null)}
      wide
    >
      {versions === null ? (
        <p className="mini-empty">{t("common.loading")}</p>
      ) : versions.length ? (
        <div className="version-list">
          {versions.map((version) => (
            <div className="version-row" key={version.id}>
              <div>
                <strong>{version.versionNumber}</strong>
                <small>
                  {version.name}
                  {version.gameVersions.length
                    ? ` · ${version.gameVersions.slice(0, 2).join(", ")}`
                    : ""}
                </small>
              </div>
              <span
                className={`tag ${version.channel === "release" ? "ok" : ""}`}
              >
                {version.channel}
              </span>
              <Button
                variant="primary"
                busy={busy.has(installKey)}
                disabled={!version.files.length}
                onClick={() => install(version)}
              >
                {t("content.install")}
              </Button>
            </div>
          ))}
        </div>
      ) : (
        <p className="mini-empty">{t("content.noVersions")}</p>
      )}
    </Modal>
  );
  return (
    <div className="content-stack">
      <section className="panel">
        <div className="section-title">
          <div>
            <h2>
              <Package size={18} />
              {t("content.title")}
            </h2>
            <p>{t("content.desc")}</p>
          </div>
          {server.serverType !== "vanilla" && (
            <Button
              onClick={() =>
                void run("folder", () =>
                  call("open_folder", {
                    id: server.id,
                    kind: plugins ? "plugins" : "mods",
                  }),
                )
              }
            >
              <FolderOpen size={16} />
              {t("common.openFolder")}
            </Button>
          )}
        </div>
        {modes.length > 1 && (
          <div className="segmented">
            <button
              className={mode === "content" ? "selected" : ""}
              onClick={() => {
                setMode("content");
                setPage(null);
              }}
            >
              {plugins ? t("content.plugins") : t("content.mods")}
            </button>
            <button
              className={mode === "modpack" ? "selected" : ""}
              onClick={() => {
                setMode("modpack");
                setPage(null);
              }}
            >
              {t("content.modpacks")}
            </button>
          </div>
        )}
        {mode === "modpack" && (
          <p className="content-note">{t("content.packNote")}</p>
        )}
        <form
          className="content-search"
          onSubmit={(event) => {
            event.preventDefault();
            if (!busy.has(searchKey)) search();
          }}
        >
          <label>
            <Search size={16} />
            <input
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder={
                mode === "modpack"
                  ? t("content.packPlaceholder")
                  : t("content.searchPlaceholder")
              }
            />
          </label>
          <Button
            type="submit"
            variant="primary"
            busy={busy.has(searchKey)}
            disabled={!query.trim()}
          >
            {t("content.search")}
          </Button>
        </form>
        {progress &&
          (busy.has(installKey) ||
            busy.has(updateKey) ||
            busy.has(packCheckKey) ||
            busy.has(packUpdateKey)) && (
          <div className="content-progress">
            <div>
              <span>{progressText(progress)}</span>
              <strong>
                {progress.totalBytes
                  ? `${Math.min(100, Math.floor((progress.downloadedBytes / progress.totalBytes) * 100))}%`
                  : ""}
              </strong>
            </div>
            <progress
              max={progress.totalBytes || undefined}
              value={progress.totalBytes ? progress.downloadedBytes : undefined}
            />
            {progress.totalBytes && (
              <small>
                {bytes(progress.downloadedBytes)} / {bytes(progress.totalBytes)}
              </small>
            )}
          </div>
        )}
        {report && (
          <div className="content-report">
            <strong>{report.name}</strong>
            <p>
              {t("content.packReport", {
                version: report.version,
                files: report.files,
                skipped: report.skipped,
                overrides: report.overrides,
              })}
            </p>
          </div>
        )}
        {page === null ? (
          <p className="mini-empty">{t("content.searchHint")}</p>
        ) : page.hits.length ? (
          <>
            <p className="content-count">
              {t("content.results", { total: page.total })}
            </p>
            <div className="project-list">
              {page.hits.map((hit) => (
                <article className="project-card" key={hit.id}>
                  <div className="project-icon">
                    {hit.iconUrl ? (
                      <img src={hit.iconUrl} alt="" loading="lazy" />
                    ) : (
                      <Puzzle size={22} />
                    )}
                  </div>
                  <div className="project-body">
                    <div className="project-head">
                      <strong>{hit.title}</strong>
                      <span className="tag">{hit.author}</span>
                    </div>
                    <p>{hit.description}</p>
                    <div className="project-meta">
                      <span>
                        <Download size={12} />
                        {compact(hit.downloads)}
                      </span>
                      {hit.categories.slice(0, 3).map((category) => (
                        <span className="tag" key={category}>
                          {category}
                        </span>
                      ))}
                      {hit.serverSide === "unsupported" && (
                        <span className="tag warning">
                          {t("content.clientOnly")}
                        </span>
                      )}
                    </div>
                  </div>
                  <Button onClick={() => browse(hit)}>
                    {t("content.chooseVersion")}
                  </Button>
                </article>
              ))}
            </div>
            {page.hits.length < page.total && (
              <div className="content-more">
                <Button
                  busy={busy.has(searchKey)}
                  onClick={() => search(page.offset + 20)}
                >
                  {t("content.loadMore")}
                </Button>
              </div>
            )}
          </>
        ) : (
          <p className="mini-empty">{t("content.noResults")}</p>
        )}
      </section>
      <section className="panel">
        <div className="section-title">
          <div>
            <h2>
              <Boxes size={18} />
              {t("content.installedTitle")}
            </h2>
            <p>{t("content.installedDesc")}</p>
          </div>
          <div className="row">
            {!plugins && (
              <Button
                disabled={!server.installed || !idle(server.runtime.status)}
                title={
                  !server.installed || !idle(server.runtime.status)
                    ? t("content.exportNeedsIdle")
                    : undefined
                }
                onClick={openExport}
              >
                <FileArchive size={15} />
                {t("content.exportPack")}
              </Button>
            )}
            <Button
              busy={busy.has(refreshKey)}
              onClick={() => void run(refreshKey, loadListing)}
            >
              <RefreshCw size={15} />
              {t("content.refresh")}
            </Button>
          </div>
        </div>
        {pack && (
          <div className="content-updates">
            <div className="content-updates-head">
              <div>
                <strong>
                  {t("content.packInstalled", { name: pack.name })}
                </strong>
                <small>
                  {t("content.packInstalledMeta", {
                    version: pack.version,
                    minecraft: pack.minecraft,
                    loader: pack.loader,
                    files: pack.files,
                    overrides: pack.overrides,
                  })}
                </small>
              </div>
              <Button busy={busy.has(packCheckKey)} onClick={checkPackUpdate}>
                <ArrowUpCircle size={15} />
                {t("content.checkPackUpdate")}
              </Button>
              {pack.canRollback && (
                <Button
                  variant="ghost"
                  disabled={busy.has(packRollbackKey)}
                  onClick={() => setPackRollbackOpen(true)}
                >
                  <Undo2 size={15} />
                  {t("content.packRollback")}
                </Button>
              )}
            </div>
            {packMessage && <p className="content-note">{packMessage}</p>}
          </div>
        )}
        {server.serverType !== "vanilla" && (
          <div className="content-updates">
            <div className="content-updates-head">
              <div>
                <strong>{t("content.updateCheckTitle")}</strong>
                <small>{t("content.updateCheckDesc")}</small>
              </div>
              <Button busy={busy.has(checkKey)} onClick={checkUpdates}>
                <ArrowUpCircle size={15} />
                {t("content.checkUpdates")}
              </Button>
            </div>
            {updates && (
              <>
                {updates.entries.some((entry) => entry.status === "update") ? (
                  <div className="content-update-list">
                    {updates.entries
                      .filter((entry) => entry.status === "update")
                      .map((entry) => (
                        <div className="content-update-row" key={entry.projectId}>
                          <div>
                            <strong>{entry.title}</strong>
                            <small>
                              {entry.currentVersion} → {entry.latestVersion}
                              {entry.channel ? ` · ${entry.channel}` : ""}
                            </small>
                            {entry.warnings.length > 0 && (
                              <div className="content-warnings">
                                {entry.warnings.map((warning) => (
                                  <span className="tag warning" key={warning}>
                                    {warningText(warning)}
                                  </span>
                                ))}
                              </div>
                            )}
                            {entry.dependencies.length > 0 && (
                              <small className="content-impact">
                                {entry.dependencies.map(impactText).join(" · ")}
                              </small>
                            )}
                          </div>
                          <Button
                            variant="primary"
                            onClick={() => setUpdateTarget(entry)}
                          >
                            {t("content.reviewUpdate")}
                          </Button>
                        </div>
                      ))}
                  </div>
                ) : (
                  <p className="mini-empty">{t("content.upToDate")}</p>
                )}
                {!updates.complete && (
                  <p className="content-note">
                    {t("content.updateCheckPartial")}
                    {updates.errors.join(" · ")}
                  </p>
                )}
                {updates.entries.some(
                  (entry) => entry.status === "unavailable",
                ) && (
                  <p className="content-note">
                    {t("content.updateUnavailableHint")}
                  </p>
                )}
              </>
            )}
            {updateMessage && <p className="content-note">{updateMessage}</p>}
          </div>
        )}
        {listing === null ? (
          <p className="mini-empty">{t("common.loading")}</p>
        ) : listing.entries.length || listing.unmanaged.length ? (
          <div className="content-rows">
            {listing.entries.map((entry) => (
              <div className="content-row" key={entry.file}>
                <div>
                  <strong>{entry.title}</strong>
                  <small>
                    {entry.file} ·{" "}
                    {t("content.version", { version: entry.versionNumber })}
                    {entry.installedAt
                      ? ` · ${new Date(entry.installedAt).toLocaleDateString()}`
                      : ""}
                  </small>
                </div>
                <span className="tag">{entry.kind}</span>
                <Button
                  variant="ghost"
                  aria-label={
                    isPinned(entry.projectId)
                      ? t("content.unpin")
                      : t("content.pin")
                  }
                  title={
                    isPinned(entry.projectId)
                      ? t("content.unpin")
                      : t("content.pin")
                  }
                  busy={busy.has(pinKey)}
                  onClick={() =>
                    togglePin(entry, !isPinned(entry.projectId))
                  }
                >
                  <Pin size={15} />
                </Button>
                {rollbackFor(entry.projectId) && (
                  <Button
                    variant="ghost"
                    aria-label={t("content.rollback")}
                    title={t("content.rollback")}
                    onClick={() =>
                      setRollbackTarget({
                        entry,
                        version: rollbackFor(entry.projectId)!.versionNumber,
                      })
                    }
                  >
                    <Undo2 size={15} />
                  </Button>
                )}
                <Button
                  variant="ghost"
                  aria-label={t("content.remove")}
                  title={t("content.remove")}
                  onClick={() => removeEntry(entry)}
                >
                  <Trash2 size={15} />
                </Button>
              </div>
            ))}
            {listing.unmanaged.map((name) => (
              <div className="content-row" key={name}>
                <div>
                  <strong>{name}</strong>
                  <small>{t("content.unmanaged")}</small>
                </div>
                <span className="tag">{t("content.manual")}</span>
                <Button
                  variant="ghost"
                  aria-label={t("content.remove")}
                  title={t("content.remove")}
                  onClick={() => removeFile(name)}
                >
                  <Trash2 size={15} />
                </Button>
              </div>
            ))}
          </div>
        ) : (
          <p className="mini-empty">{t("content.emptyInstalled")}</p>
        )}
      </section>
      <DependencyCheck
        key={`${listing?.entries.map((e) => e.versionId).join(",")}-${listing?.unmanaged.join(",")}`}
        id={server.id}
        run={run}
        busy={busy}
      />
      {installModal}
      {updateTarget && (
        <Modal
          title={t("content.updateTitle", { name: updateTarget.title })}
          onClose={() => {
            if (!busy.has(updateKey)) setUpdateTarget(null);
          }}
          wide
        >
          <div className="content-report">
            <strong>
              {updateTarget.currentVersion} → {updateTarget.latestVersion}
            </strong>
            <p>
              {t("content.updateReview", {
                current: updateTarget.currentVersion,
                latest: updateTarget.latestVersion ?? "",
              })}
            </p>
          </div>
          {updateTarget.warnings.length > 0 && (
            <div className="content-warnings">
              {updateTarget.warnings.map((warning) => (
                <span className="tag warning" key={warning}>
                  {warningText(warning)}
                </span>
              ))}
            </div>
          )}
          {updateTarget.changelog ? (
            <div className="content-changelog">
              <strong>{t("content.changelog")}</strong>
              <pre>{updateTarget.changelog}</pre>
            </div>
          ) : (
            <p className="content-note">{t("content.noChangelog")}</p>
          )}
          {updateTarget.dependencies.length > 0 && (
            <div className="content-changelog">
              <strong>{t("content.dependencyImpact")}</strong>
              <ul>
                {updateTarget.dependencies.map((dependency) => (
                  <li key={dependency.projectId}>
                    <span
                      className={`tag ${dependency.status === "missing" || dependency.status === "incompatible" ? "warning" : ""}`}
                    >
                      {dependency.dependencyType}
                    </span>
                    {impactText(dependency)}
                  </li>
                ))}
              </ul>
            </div>
          )}
          <p className="content-note">{t("content.updateNote")}</p>
          <div className="modal-actions">
            <Button
              onClick={() => setUpdateTarget(null)}
              disabled={busy.has(updateKey)}
            >
              {t("common.cancel")}
            </Button>
            <Button
              variant="primary"
              busy={busy.has(updateKey)}
              onClick={() => applyUpdate(updateTarget)}
            >
              <ArrowUpCircle size={15} />
              {t("content.updateNow")}
            </Button>
          </div>
        </Modal>
      )}
      {rollbackTarget && (
        <Modal
          title={t("content.rollbackTitle", {
            name: rollbackTarget.entry.title,
          })}
          onClose={() => {
            if (!busy.has(rollbackKey)) setRollbackTarget(null);
          }}
        >
          <p>
            {t("content.rollbackBody", { version: rollbackTarget.version })}
          </p>
          <div className="modal-actions">
            <Button
              onClick={() => setRollbackTarget(null)}
              disabled={busy.has(rollbackKey)}
            >
              {t("common.cancel")}
            </Button>
            <Button
              variant="danger"
              busy={busy.has(rollbackKey)}
              onClick={rollbackUpdate}
            >
              <Undo2 size={15} />
              {t("content.rollback")}
            </Button>
          </div>
        </Modal>
      )}
      {packPreview && (
        <Modal
          title={t("content.packUpdateTitle", { name: packPreview.name })}
          onClose={() => {
            if (!busy.has(packUpdateKey)) setPackPreview(null);
          }}
          wide
        >
          <div className="content-report">
            <strong>
              {packPreview.fromVersion} → {packPreview.toVersion}
              {packPreview.channel ? ` · ${packPreview.channel}` : ""}
            </strong>
            <p>
              {t("content.packUpdateSummary", {
                added: packPreview.added.length,
                removed: packPreview.removed.length,
                changed: packPreview.changed.length,
                unchanged: packPreview.unchanged,
                overrides: packPreview.overridesChanged,
              })}
            </p>
          </div>
          {packPreview.changelog ? (
            <div className="content-changelog">
              <strong>{t("content.changelog")}</strong>
              <pre>{packPreview.changelog}</pre>
            </div>
          ) : (
            <p className="content-note">{t("content.noChangelog")}</p>
          )}
          <div className="pack-diff">
            {(
              [
                ["added", packPreview.added],
                ["changed", packPreview.changed],
                ["removed", packPreview.removed],
              ] as const
            ).map(([kind, paths]) => (
              <div key={kind}>
                <strong>
                  {t(`content.packDiff.${kind}` as MessageKey)}
                </strong>
                {paths.length ? (
                  <ul>
                    {paths.map((path) => (
                      <li key={path}>{path}</li>
                    ))}
                  </ul>
                ) : (
                  <small>{t("content.packDiff.none")}</small>
                )}
              </div>
            ))}
          </div>
          {packPreview.truncated && (
            <p className="content-note">{t("content.packDiffTruncated")}</p>
          )}
          <p className="content-note">{t("content.packUpdateNote")}</p>
          <div className="modal-actions">
            <Button
              onClick={() => setPackPreview(null)}
              disabled={busy.has(packUpdateKey)}
            >
              {t("common.cancel")}
            </Button>
            <Button
              variant="primary"
              busy={busy.has(packUpdateKey)}
              onClick={applyPackUpdate}
            >
              <ArrowUpCircle size={15} />
              {t("content.packUpdateNow")}
            </Button>
          </div>
        </Modal>
      )}
      {packRollbackOpen && pack && (
        <Modal
          title={t("content.packRollbackTitle", { name: pack.name })}
          onClose={() => {
            if (!busy.has(packRollbackKey)) setPackRollbackOpen(false);
          }}
        >
          <p>
            {t("content.packRollbackBody", {
              version: pack.rollbackVersion ?? pack.version,
            })}
          </p>
          <div className="modal-actions">
            <Button
              onClick={() => setPackRollbackOpen(false)}
              disabled={busy.has(packRollbackKey)}
            >
              {t("common.cancel")}
            </Button>
            <Button
              variant="danger"
              busy={busy.has(packRollbackKey)}
              onClick={rollbackPack}
            >
              <Undo2 size={15} />
              {t("content.packRollback")}
            </Button>
          </div>
        </Modal>
      )}
      {exportOpen && (
        <Modal
          title={t("content.exportTitle")}
          onClose={() => {
            if (!busy.has(exportKey)) setExportOpen(false);
          }}
        >
          <p>{t("content.exportHint")}</p>
          <div className="rule-grid">
            <Field label={t("content.exportName")}>
              <input
                value={exportForm.name}
                maxLength={64}
                onChange={(event) =>
                  setExportForm((previous) => ({
                    ...previous,
                    name: event.target.value,
                  }))
                }
              />
            </Field>
            <Field label={t("content.exportVersion")}>
              <input
                value={exportForm.version}
                maxLength={32}
                onChange={(event) =>
                  setExportForm((previous) => ({
                    ...previous,
                    version: event.target.value,
                  }))
                }
              />
            </Field>
          </div>
          <Field label={t("content.exportPath")} hint={t("content.exportPathHint")}>
            <div className="row">
              <input
                value={exportForm.path}
                readOnly
                placeholder={t("content.exportNoPath")}
              />
              <Button
                busy={busy.has(exportPickKey)}
                disabled={busy.has(exportKey)}
                onClick={pickExportPath}
              >
                <FolderOpen size={15} />
                {t("content.exportChoose")}
              </Button>
            </div>
          </Field>
          {exportResult && (
            <div className="content-report">
              <strong>{t("content.exportDone")}</strong>
              <p>{exportResult.path}</p>
              <small>
                {t("content.exportStats", {
                  files: exportResult.files,
                  overrides: exportResult.overrides,
                  skipped: exportResult.skipped,
                  size: bytes(exportResult.sizeBytes),
                })}
              </small>
            </div>
          )}
          <p className="content-note">{t("content.exportNote")}</p>
          <div className="modal-actions">
            <Button
              onClick={() => setExportOpen(false)}
              disabled={busy.has(exportKey)}
            >
              {exportResult ? t("common.close") : t("common.cancel")}
            </Button>
            <Button
              variant="primary"
              busy={busy.has(exportKey)}
              disabled={!exportForm.path.trim()}
              onClick={exportPack}
            >
              <FileArchive size={15} />
              {busy.has(exportKey)
                ? t("content.exporting")
                : t("content.exportRun")}
            </Button>
          </div>
        </Modal>
      )}
      {pending && (
        <Modal title={pending.title} onClose={() => setPending(null)}>
          <p>{pending.body}</p>
          <div className="modal-actions">
            <Button onClick={() => setPending(null)}>
              {t("common.cancel")}
            </Button>
            <Button
              variant="danger"
              busy={busy.has("content-remove")}
              onClick={() =>
                void run("content-remove", async () => {
                  await pending.action();
                  setPending(null);
                })
              }
            >
              <Trash2 size={15} />
              {t("content.remove")}
            </Button>
          </div>
        </Modal>
      )}
    </div>
  );
}
