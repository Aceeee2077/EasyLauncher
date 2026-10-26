import { useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  BookOpen,
  Download,
  Package,
  Search,
  Server as ServerIcon,
  Sparkles,
  Users,
} from "lucide-react";
import { Button, Field, Modal, bytes } from "../components/ui";
import { call, desktop } from "../services/api";
import { useI18n, type MessageKey } from "../i18n";
import type {
  ContentProgress,
  ProjectDetail,
  ProjectHit,
  ProjectPage,
  ProjectVersion,
  Server,
  ServerView,
} from "../types";
import type { TaskProps } from "../utils/task";

type Kind = "modpack" | "mod";
type InstallTarget = { hit: ProjectHit; version: ProjectVersion };
type CreateTarget = { hit: ProjectHit; version: ProjectVersion };

const compact = (n: number) =>
  n >= 1_000_000
    ? `${(n / 1_000_000).toFixed(1)}M`
    : n >= 1_000
      ? `${(n / 1_000).toFixed(1)}k`
      : String(n);

const sorts = [
  "relevance",
  "downloads",
  "follows",
  "newest",
  "updated",
] as const;

const loaders = ["fabric", "forge", "neoforge", "quilt"] as const;

const vanillaLike = (loaders: string[]) =>
  loaders.every((loader) =>
    ["", "minecraft", "vanilla"].includes(loader.toLowerCase()),
  );

const supportsOneClick = (version: ProjectVersion) =>
  version.loaders.some((loader) => loader.toLowerCase() === "fabric") ||
  vanillaLike(version.loaders);

const targetTypes = (kind: Kind, version: ProjectVersion): string[] => {
  const fabric = version.loaders.some(
    (loader) => loader.toLowerCase() === "fabric",
  );
  if (kind === "mod") return fabric ? ["fabric"] : [];
  if (fabric) return ["fabric"];
  if (vanillaLike(version.loaders)) return ["vanilla"];
  return [];
};

export function Discover({
  servers,
  run,
  busy,
  onCreated,
}: TaskProps & {
  servers: ServerView[];
  onCreated: (server: Server) => void;
}) {
  const { t } = useI18n();
  const [kind, setKind] = useState<Kind>("modpack");
  const [query, setQuery] = useState("");
  const [gameVersion, setGameVersion] = useState("");
  const [loader, setLoader] = useState("");
  const [sort, setSort] = useState<(typeof sorts)[number]>("relevance");
  const [versions, setVersions] = useState<string[]>([]);
  const [page, setPage] = useState<ProjectPage | null>(null);
  const [detailHit, setDetailHit] = useState<ProjectHit | null>(null);
  const [detail, setDetail] = useState<ProjectDetail | null>(null);
  const [releases, setReleases] = useState<ProjectVersion[] | null>(null);
  const [showAllVersions, setShowAllVersions] = useState(false);
  const [installTarget, setInstallTarget] = useState<InstallTarget | null>(
    null,
  );
  const [targetServer, setTargetServer] = useState("");
  const [createTarget, setCreateTarget] = useState<CreateTarget | null>(null);
  const [form, setForm] = useState({
    name: "",
    port: "",
    memoryMaxMb: 2048,
    onlineMode: true,
    eula: false,
  });
  const [progress, setProgress] = useState<ContentProgress | null>(null);
  const [message, setMessage] = useState("");
  const searchKey = `discover-search-${kind}`;
  const searchSeq = useRef(0);
  const detailSeq = useRef(0);
  const installKey = "discover-install";
  const createKey = "discover-create";

  useEffect(() => {
    if (!desktop) return;
    void call("list_versions", { software: "vanilla", snapshots: false })
      .then((list) =>
        setVersions(
          list
            .filter((version) => version.type === "release")
            .slice(0, 80)
            .map((version) => version.id),
        ),
      )
      .catch(() => setVersions([]));
  }, []);

  useEffect(() => {
    const subscription = listen<ContentProgress>("content-progress", (event) => {
      setProgress(event.payload);
    });
    return () => {
      void subscription.then((stop) => stop());
    };
  }, []);

  const search = (offset = 0) => {
    const seq = ++searchSeq.current;
    void run(searchKey, async () => {
      const result = await call("modrinth_search", {
        query,
        projectType: kind,
        gameVersion: gameVersion || undefined,
        loader: loader || undefined,
        sort,
        offset,
      });
      if (seq !== searchSeq.current) return;
      setPage((previous) =>
        previous && offset > 0
          ? {
              hits: [...previous.hits, ...result.hits],
              total: result.total,
              offset: result.offset,
              limit: result.limit,
            }
          : result,
      );
    });
  };

  useEffect(() => {
    if (desktop) search(0);
    // A new tab starts a fresh query; filters apply when the user searches.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [kind]);

  const openDetail = (hit: ProjectHit) => {
    const seq = ++detailSeq.current;
    setDetailHit(hit);
    setDetail(null);
    setReleases(null);
    setShowAllVersions(false);
    void run(`discover-detail-${hit.id}`, async () => {
      const [project, list] = await Promise.all([
        call("modrinth_project", { projectId: hit.id }),
        call("modrinth_versions", {
          projectId: hit.id,
          gameVersion: gameVersion || undefined,
          loader: loader || undefined,
        }),
      ]);
      if (seq !== detailSeq.current) return;
      setDetail(project);
      setReleases(list);
    });
  };

  const openInstall = (hit: ProjectHit, version: ProjectVersion) => {
    const types = targetTypes(kind, version);
    const first = servers.find(
      (server) =>
        server.installed &&
        types.includes(server.serverType) &&
        (version.gameVersions.length === 0 ||
          version.gameVersions.includes(server.minecraftVersion)),
    );
    setTargetServer(first?.id ?? "");
    setInstallTarget({ hit, version });
  };

  const compatibleServers = useMemo(() => {
    if (!installTarget) return [];
    const types = targetTypes(kind, installTarget.version);
    return servers.filter(
      (server) =>
        server.installed &&
        types.includes(server.serverType) &&
        (installTarget.version.gameVersions.length === 0 ||
          installTarget.version.gameVersions.includes(server.minecraftVersion)),
    );
  }, [installTarget, kind, servers]);

  const install = () => {
    if (!installTarget || !targetServer) return;
    void run(installKey, async () => {
      setProgress(null);
      if (kind === "modpack") {
        await call("modpack_install", {
          id: targetServer,
          projectId: installTarget.hit.id,
          versionId: installTarget.version.id,
        });
      } else {
        await call("modrinth_install", {
          id: targetServer,
          projectId: installTarget.hit.id,
          versionId: installTarget.version.id,
          kind: "mod",
        });
      }
      setMessage(t("discover.installed", { name: installTarget.hit.title }));
      setInstallTarget(null);
    });
  };

  const openCreate = (hit: ProjectHit, version: ProjectVersion) => {
    setForm({
      name: hit.title.slice(0, 64),
      port: "",
      memoryMaxMb: 2048,
      onlineMode: true,
      eula: false,
    });
    setCreateTarget({ hit, version });
  };

  const create = () => {
    if (!createTarget || !form.eula) return;
    void run(createKey, async () => {
      setProgress(null);
      const result = await call("modpack_create_server", {
        input: {
          projectId: createTarget.hit.id,
          versionId: createTarget.version.id,
          name: form.name.trim() || undefined,
          port: form.port.trim() ? Number(form.port) : undefined,
          memoryMinMb: 512,
          memoryMaxMb: form.memoryMaxMb,
          onlineMode: form.onlineMode,
          eulaAccepted: true,
        },
      });
      setCreateTarget(null);
      setMessage(t("discover.created", { name: result.server.name }));
      onCreated(result.server);
    });
  };

  const active = busy.has(installKey) || busy.has(createKey);
  const activeProgress =
    active && progress ? (
      <div className="content-progress">
        <div>
          <span>{progress.file}</span>
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
    ) : null;
  const shownVersions = releases
    ? showAllVersions
      ? releases
      : releases.slice(0, 30)
    : null;

  return (
    <div className="stack">
      <section className="panel">
        <div className="section-title">
          <div>
            <h2>
              <Sparkles size={18} /> {t("discover.title")}
            </h2>
            <p>{t("discover.subtitle")}</p>
          </div>
          <div className="segmented">
            <button
              className={kind === "modpack" ? "selected" : ""}
              onClick={() => {
                setKind("modpack");
                setPage(null);
                setDetailHit(null);
              }}
            >
              {t("discover.tab.modpacks")}
            </button>
            <button
              className={kind === "mod" ? "selected" : ""}
              onClick={() => {
                setKind("mod");
                setPage(null);
                setDetailHit(null);
              }}
            >
              {t("discover.tab.mods")}
            </button>
          </div>
        </div>

        <form
          className="content-search"
          onSubmit={(event) => {
            event.preventDefault();
            if (!busy.has(searchKey)) search(0);
          }}
        >
          <label>
            <Search size={16} />
            <input
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder={t("discover.searchPlaceholder")}
            />
          </label>
          <Button
            type="submit"
            variant="primary"
            busy={busy.has(searchKey)}
          >
            {t("discover.search")}
          </Button>
        </form>

        <div className="discover-filters">
          <label>
            <span>{t("discover.gameVersion")}</span>
            <select
              value={gameVersion}
              onChange={(event) => setGameVersion(event.target.value)}
            >
              <option value="">{t("discover.allVersions")}</option>
              {versions.map((version) => (
                <option key={version} value={version}>
                  {version}
                </option>
              ))}
            </select>
          </label>
          <label>
            <span>{t("discover.loader")}</span>
            <select
              value={loader}
              onChange={(event) => setLoader(event.target.value)}
            >
              <option value="">{t("discover.allLoaders")}</option>
              {loaders.map((value) => (
                <option key={value} value={value}>
                  {value}
                </option>
              ))}
            </select>
          </label>
          <label>
            <span>{t("discover.sort")}</span>
            <select
              value={sort}
              onChange={(event) =>
                setSort(event.target.value as (typeof sorts)[number])
              }
            >
              {sorts.map((value) => (
                <option key={value} value={value}>
                  {t(`discover.sort.${value}` as MessageKey)}
                </option>
              ))}
            </select>
          </label>
        </div>

        {message && (
          <p className="info-strip" role="status">
            {message}
          </p>
        )}

        {activeProgress}

        {page === null ? (
          <p className="mini-empty">{t("discover.searchHint")}</p>
        ) : page.hits.length ? (
          <>
            <p className="content-count">
              {t("discover.results", { total: page.total })}
            </p>
            <div className="project-list">
              {page.hits.map((hit) => (
                <article className="project-card" key={hit.id}>
                  <div className="project-icon">
                    {hit.iconUrl ? (
                      <img src={hit.iconUrl} alt="" loading="lazy" />
                    ) : (
                      <Package size={22} />
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
                          {t("discover.clientOnly")}
                        </span>
                      )}
                    </div>
                  </div>
                  <Button onClick={() => openDetail(hit)}>
                    <BookOpen size={15} />
                    {t("discover.details")}
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
                  {t("discover.loadMore")}
                </Button>
              </div>
            )}
          </>
        ) : (
          <p className="mini-empty">{t("discover.noResults")}</p>
        )}
      </section>

      {detailHit && (
        <Modal
          title={detailHit.title}
          wide
          onClose={() => setDetailHit(null)}
        >
          <div className="discover-detail-head">
            <div className="project-icon large">
              {detail?.iconUrl || detailHit.iconUrl ? (
                <img
                  src={detail?.iconUrl || detailHit.iconUrl || ""}
                  alt=""
                />
              ) : (
                <Package size={28} />
              )}
            </div>
            <div>
              <strong>
                {detail?.title || detailHit.title}{" "}
                <span className="tag">{detailHit.author}</span>
              </strong>
              <p>{detail?.description || detailHit.description}</p>
              <div className="project-meta">
                <span>
                  <Download size={12} />
                  {compact(detail?.downloads ?? detailHit.downloads)}
                </span>
                <span>
                  <Users size={12} />
                  {compact(detail?.followers ?? detailHit.follows)}
                </span>
                {detail?.licenseName && (
                  <span>
                    {t("discover.license")}: {detail.licenseName}
                  </span>
                )}
              </div>
            </div>
          </div>

          {detail?.gallery.length ? (
            <div className="discover-gallery">
              {detail.gallery
                .filter((image) => image.featured)
                .concat(detail.gallery.filter((image) => !image.featured))
                .slice(0, 3)
                .map((image) => (
                  <img
                    key={image.url}
                    src={image.url}
                    alt={image.title || ""}
                    loading="lazy"
                  />
                ))}
            </div>
          ) : null}

          <div className="discover-tags">
            {[...(detail?.categories ?? []), ...(detail?.loaders ?? [])].map(
              (tag, index) => (
                <span className="tag" key={`${tag}-${index}`}>
                  {tag}
                </span>
              ),
            )}
            {(detail?.gameVersions ?? []).slice(-8).map((version) => (
              <span className="tag" key={version}>
                MC {version}
              </span>
            ))}
          </div>

          <div className="discover-body">
            <strong>{t("discover.description")}</strong>
            <pre>
              {detail?.body?.trim() || t("discover.noDescription")}
            </pre>
          </div>

          <div className="section-title compact-title">
            <div>
              <h2>
                <ServerIcon size={17} /> {t("discover.versions")}
              </h2>
              {releases && (
                <p>{t("discover.versionCount", { count: releases.length })}</p>
              )}
            </div>
          </div>

          {!releases ? (
            <p className="mini-empty">{t("discover.searchHint")}</p>
          ) : releases.length === 0 ? (
            <p className="mini-empty">{t("discover.noVersions")}</p>
          ) : (
            <div className="version-list">
              {shownVersions?.map((version) => (
                <div className="version-row" key={version.id}>
                  <div>
                    <strong>
                      {version.versionNumber} · {version.name}
                    </strong>
                    <small>
                      {version.channel} ·{" "}
                      {version.loaders.join(", ") || "vanilla"} ·{" "}
                      {version.gameVersions.slice(0, 4).join(", ")} ·{" "}
                      {compact(version.downloads)} downloads
                    </small>
                  </div>
                  <Button
                    disabled={active || targetTypes(kind, version).length === 0}
                    onClick={() => openInstall(detailHit, version)}
                  >
                    <Download size={14} />
                    {t("discover.installTo")}
                  </Button>
                  {kind === "modpack" && (
                    <Button
                      variant="primary"
                      disabled={active || !supportsOneClick(version)}
                      title={
                        supportsOneClick(version)
                          ? undefined
                          : t("discover.unsupportedLoader")
                      }
                      onClick={() => openCreate(detailHit, version)}
                    >
                      <ServerIcon size={14} />
                      {t("discover.oneClick")}
                    </Button>
                  )}
                </div>
              ))}
            </div>
          )}
          {releases && releases.length > 30 && !showAllVersions && (
            <div className="content-more">
              <Button onClick={() => setShowAllVersions(true)}>
                {t("discover.showMore")}
              </Button>
            </div>
          )}
          {kind === "mod" && (
            <p className="content-note">{t("discover.installModHint")}</p>
          )}
          {kind === "modpack" && (
            <p className="content-note">{t("discover.fabricOnly")}</p>
          )}
        </Modal>
      )}

      {installTarget && (
        <Modal
          title={t("discover.installTitle", { name: installTarget.hit.title })}
          onClose={() => {
            if (!busy.has(installKey)) setInstallTarget(null);
          }}
        >
          <p>
            {installTarget.version.versionNumber} ·{" "}
            {installTarget.version.gameVersions.slice(0, 4).join(", ")}
          </p>
          {activeProgress}
          {compatibleServers.length ? (
            <Field label={t("discover.chooseServer")}>
              <select
                value={targetServer}
                onChange={(event) => setTargetServer(event.target.value)}
              >
                {compatibleServers.map((server) => (
                  <option key={server.id} value={server.id}>
                    {server.name} · {server.serverType}{" "}
                    {server.minecraftVersion}
                  </option>
                ))}
              </select>
            </Field>
          ) : (
            <p className="content-note">
              {t("discover.noCompatibleServer")}
            </p>
          )}
          <div className="modal-actions">
            <Button
              onClick={() => setInstallTarget(null)}
              disabled={busy.has(installKey)}
            >
              {t("common.cancel")}
            </Button>
            <Button
              variant="primary"
              busy={busy.has(installKey)}
              disabled={!targetServer}
              onClick={install}
            >
              <Download size={15} />
              {t("discover.install")}
            </Button>
          </div>
        </Modal>
      )}

      {createTarget && (
        <Modal
          title={t("discover.oneClickTitle", { name: createTarget.hit.title })}
          onClose={() => {
            if (!busy.has(createKey)) setCreateTarget(null);
          }}
        >
          <p>{t("discover.createHint")}</p>
          {activeProgress}
          <div className="rule-grid">
            <Field label={t("discover.serverName")}>
              <input
                value={form.name}
                maxLength={64}
                onChange={(event) =>
                  setForm((previous) => ({
                    ...previous,
                    name: event.target.value,
                  }))
                }
              />
            </Field>
            <Field label={t("discover.port")} hint={t("discover.portHint")}>
              <input
                type="number"
                min={1}
                max={65535}
                value={form.port}
                onChange={(event) =>
                  setForm((previous) => ({
                    ...previous,
                    port: event.target.value,
                  }))
                }
              />
            </Field>
            <Field label={t("discover.memory")}>
              <input
                type="number"
                min={512}
                max={65536}
                step={256}
                value={form.memoryMaxMb}
                onChange={(event) =>
                  setForm((previous) => ({
                    ...previous,
                    memoryMaxMb: Number(event.target.value),
                  }))
                }
              />
            </Field>
          </div>
          <label className="check-row">
            <input
              type="checkbox"
              checked={form.onlineMode}
              onChange={(event) =>
                setForm((previous) => ({
                  ...previous,
                  onlineMode: event.target.checked,
                }))
              }
            />
            <span>
              {t("discover.onlineMode")}
              <small>{t("discover.onlineModeHint")}</small>
            </span>
          </label>
          <label className="check-row">
            <input
              type="checkbox"
              checked={form.eula}
              onChange={(event) =>
                setForm((previous) => ({
                  ...previous,
                  eula: event.target.checked,
                }))
              }
            />
            <span>{t("discover.eula")}</span>
          </label>
          <div className="modal-actions">
            <Button
              onClick={() => setCreateTarget(null)}
              disabled={busy.has(createKey)}
            >
              {t("common.cancel")}
            </Button>
            <Button
              variant="primary"
              busy={busy.has(createKey)}
              disabled={!form.eula}
              onClick={create}
            >
              <Sparkles size={15} />
              {busy.has(createKey) ? t("discover.creating") : t("discover.create")}
            </Button>
          </div>
        </Modal>
      )}
    </div>
  );
}
