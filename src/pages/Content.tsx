import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  Boxes,
  Download,
  FolderOpen,
  Package,
  Puzzle,
  RefreshCw,
  Search,
  Trash2,
} from "lucide-react";
import { Button, Modal, bytes } from "../components/ui";
import { DependencyCheck } from "../components/DependencyCheck";
import { call } from "../services/api";
import { useI18n } from "../i18n";
import type {
  ContentEntry,
  ContentListing,
  ContentProgress,
  ModpackReport,
  ProjectHit,
  ProjectVersion,
  ServerView,
} from "../types";
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
  const kind = server.serverType === "paper" ? "plugin" : "mod";
  const loader = server.serverType === "paper" ? "paper" : "fabric";
  const modes: Mode[] =
    server.serverType === "vanilla"
      ? ["modpack"]
      : server.serverType === "paper"
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
  const [pending, setPending] = useState<Pending | null>(null);
  const refreshKey = `content-list-${server.id}`;
  const searchKey = `content-search-${server.id}`;
  const installKey = `content-install-${server.id}`;
  const loadListing = async () =>
    setListing(await call("content_list", { id: server.id }));
  useEffect(() => {
    void run(refreshKey, loadListing);
    setListing(null);
    setPage(null);
    setActive(null);
    setVersions(null);
    setProgress(null);
    setReport(null);
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
  const progressText = (value: ContentProgress) => {
    const vars = {
      file: value.file,
      done: value.completed,
      total: value.total,
    };
    if (value.stage === "download") return t("content.progress.download", vars);
    if (value.stage === "overrides")
      return t("content.progress.overrides", vars);
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
                    kind: server.serverType === "paper" ? "plugins" : "mods",
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
              {server.serverType === "paper"
                ? t("content.plugins")
                : t("content.mods")}
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
        {progress && busy.has(installKey) && (
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
          <Button
            busy={busy.has(refreshKey)}
            onClick={() => void run(refreshKey, loadListing)}
          >
            <RefreshCw size={15} />
            {t("content.refresh")}
          </Button>
        </div>
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
