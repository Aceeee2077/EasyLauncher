import { useEffect, useRef, useState } from "react";
import { Copy, Download, FolderOpen, Share2 } from "lucide-react";
import { Button, Field } from "./ui";
import { call, desktop } from "../services/api";
import { useI18n } from "../i18n";
import type { ServerView } from "../types";
import type { TaskProps } from "../utils/task";

export function ShareCard({
  server,
  address: suggested,
  run,
  busy,
}: TaskProps & { server: ServerView; address: string }) {
  const { t, locale } = useI18n();
  const canvas = useRef<HTMLCanvasElement>(null);
  const [customAddress, setAddress] = useState<string | null>(null);
  const [description, setDescription] = useState("");
  const [message, setMessage] = useState("");
  const address = customAddress ?? suggested;
  const key = `share-card-${server.id}`;
  useEffect(() => {
    const ctx = canvas.current?.getContext("2d");
    if (!ctx) return;
    const gradient = ctx.createLinearGradient(0, 0, 1200, 720);
    gradient.addColorStop(0, "#122e35");
    gradient.addColorStop(1, "#225849");
    ctx.fillStyle = gradient;
    ctx.fillRect(0, 0, 1200, 720);
    // Layered mountain silhouettes are drawn locally, keeping exports self-contained.
    for (let layer = 0; layer < 3; layer++) {
      ctx.fillStyle = ["#38756b55", "#4a8c7355", "#9cae8155"][layer];
      ctx.beginPath();
      ctx.moveTo(0, 720);
      for (let x = 0; x <= 1200; x += 60)
        ctx.lineTo(
          x,
          370 +
            layer * 80 +
            Math.sin(x / 170 + layer * 2) * 85 +
            Math.cos(x / 90) * 25,
        );
      ctx.lineTo(1200, 720);
      ctx.closePath();
      ctx.fill();
    }
    ctx.strokeStyle = "#d9bd8070";
    ctx.lineWidth = 2;
    ctx.strokeRect(32, 32, 1136, 656);
    const text = (
      value: string,
      x: number,
      y: number,
      size: number,
      color: string,
      width = 1050,
    ) => {
      ctx.font = `${size >= 50 ? "600" : "400"} ${size}px "Microsoft YaHei", sans-serif`;
      ctx.fillStyle = color;
      let clipped = value;
      while (ctx.measureText(clipped).width > width && clipped.length > 1)
        clipped = clipped.slice(0, -1);
      ctx.fillText(
        clipped === value ? value : `${clipped.slice(0, -1)}…`,
        x,
        y,
      );
    };
    text("EASYLAUNCHER  /  MINECRAFT JAVA", 76, 108, 22, "#dfc78e");
    text(server.name, 76, 225, 64, "#fff9e9");
    text(description || t("share.defaultDescription"), 78, 284, 27, "#dce8dd");
    text(
      `${server.serverType.toUpperCase()}   ·   ${server.minecraftVersion}`,
      78,
      347,
      25,
      "#dfc78e",
    );
    ctx.fillStyle = "#091e27ba";
    ctx.beginPath();
    ctx.roundRect(76, 422, 1048, 130, 16);
    ctx.fill();
    text(t("share.address"), 106, 461, 20, "#a8c8bc");
    text(address || "—", 106, 514, 36, "#ffffff", 980);
    text(t("share.javaEdition"), 78, 620, 21, "#e9eddf");
    text(t("share.reachability"), 78, 654, 17, "#b4cbbb");
  }, [
    server.name,
    server.serverType,
    server.minecraftVersion,
    address,
    description,
    locale,
    t,
  ]);
  const png = () =>
    new Promise<Blob>((resolve, reject) => {
      if (!canvas.current) {
        reject(new Error(t("share.failed")));
        return;
      }
      canvas.current.toBlob(
        (blob) => (blob ? resolve(blob) : reject(new Error(t("share.failed")))),
        "image/png",
      );
    });
  return (
    <section className="panel">
      <div className="section-title">
        <div>
          <h2>
            <Share2 size={18} />
            {t("share.title")}
          </h2>
          <p>{t("share.desc")}</p>
        </div>
      </div>
      <div className="form-grid">
        <Field label={t("share.address")}>
          <input
            value={address}
            maxLength={128}
            onChange={(e) => {
              setAddress(e.target.value);
              setMessage("");
            }}
          />
        </Field>
        <Field label={t("share.description")}>
          <input
            value={description}
            maxLength={80}
            placeholder={t("share.defaultDescription")}
            onChange={(e) => setDescription(e.target.value)}
          />
        </Field>
      </div>
      <canvas
        ref={canvas}
        width={1200}
        height={720}
        className="share-canvas"
        role="img"
        aria-label={`${server.name} · ${server.minecraftVersion} · ${address}`}
      />
      <div className="row share-actions">
        <Button
          disabled={!address}
          busy={busy.has(`${key}-save`)}
          onClick={() =>
            void run(`${key}-save`, async () => {
              const blob = await png();
              if (desktop) {
                const path = await call("save_share_card", {
                  bytes: Array.from(new Uint8Array(await blob.arrayBuffer())),
                });
                setMessage(t("share.saved", { path }));
              } else {
                const url = URL.createObjectURL(blob);
                const a = document.createElement("a");
                a.href = url;
                a.download = "easylauncher-server.png";
                a.click();
                window.setTimeout(() => URL.revokeObjectURL(url), 1000);
                setMessage(t("share.downloaded"));
              }
            })
          }
        >
          <Download size={16} />
          {t("share.save")}
        </Button>
        <Button
          disabled={!address}
          busy={busy.has(`${key}-copy`)}
          onClick={() =>
            void run(`${key}-copy`, async () => {
              // Start clipboard write in the click activation; defer PNG generation inside ClipboardItem.
              if (!navigator.clipboard?.write || !globalThis.ClipboardItem)
                throw new Error(t("share.copyUnavailable"));
              await navigator.clipboard.write([
                new ClipboardItem({ "image/png": png() }),
              ]);
              setMessage(t("share.copied"));
            })
          }
        >
          <Copy size={16} />
          {t("share.copy")}
        </Button>
        {desktop && (
          <Button
            onClick={() =>
              void run(`${key}-folder`, () => call("open_share_cards"))
            }
          >
            <FolderOpen size={16} />
            {t("share.folder")}
          </Button>
        )}
      </div>
      {message && (
        <p className="success-text break-anywhere" role="status">
          {message}
        </p>
      )}
    </section>
  );
}
