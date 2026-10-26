import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { LanguageProvider } from "../i18n";
import { ActivityWorkbench } from "./ActivityWorkbench";
import type { ActivityReport } from "../types";

const api = vi.hoisted(() => vi.fn());
vi.mock("../services/api", () => ({ call: api }));
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  api.mockReset();
});
const report: ActivityReport = {
  source: "latest.log",
  scannedLines: 25,
  truncated: false,
  players: ["Alex", "Steve"],
  hours: Array.from({ length: 24 }, (_, i) =>
    i === 12 ? 2 : i === 13 ? 1 : 0,
  ),
  events: [
    {
      line: 1,
      time: "12:00:01",
      hour: 12,
      kind: "join",
      player: "Alex",
      message: "Alex joined the game",
    },
    {
      line: 2,
      time: "12:00:02",
      hour: 12,
      kind: "death",
      player: "Steve",
      message: "Steve was slain by Zombie",
    },
    {
      line: 3,
      time: "13:00:01",
      hour: 13,
      kind: "warning",
      player: null,
      message: "Can't keep up!",
    },
  ],
};
function mount(data = report) {
  api.mockImplementation(async (command) =>
    command === "activity_library"
      ? {
          files: [
            { name: "latest.log", bytes: 100 },
            { name: "old.log.gz", bytes: 50 },
          ],
          omitted: 0,
        }
      : data,
  );
  render(
    <LanguageProvider>
      <ActivityWorkbench
        id="server"
        run={async (_, fn) => fn()}
        busy={new Set()}
      />
    </LanguageProvider>,
  );
}
async function analyze() {
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "分析所选日志" })).toBeEnabled(),
  );
  fireEvent.click(screen.getByRole("button", { name: "分析所选日志" }));
  await screen.findByText("Alex joined the game");
}
describe("log insights workbench", () => {
  it("requires explicit analysis and combines player, hour and text filters", async () => {
    mount();
    await analyze();
    expect(api).toHaveBeenCalledWith("activity_analyze", {
      id: "server",
      name: "latest.log",
    });
    fireEvent.change(screen.getByLabelText("玩家"), {
      target: { value: "Alex" },
    });
    expect(
      screen.queryByText("Steve was slain by Zombie"),
    ).not.toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("搜索事件内容"), {
      target: { value: "missing" },
    });
    expect(
      screen.getByText("没有匹配的事件。可清除筛选或选择其他日志。"),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "清除筛选" }));
    fireEvent.click(screen.getByRole("button", { name: "13 时：1 个事件" }));
    expect(screen.getByText("Can't keep up!")).toBeInTheDocument();
    expect(screen.queryByText("Alex joined the game")).not.toBeInTheDocument();
  });
  it("bookmarks evidence and exports only the filtered selection", async () => {
    mount();
    await analyze();
    fireEvent.click(screen.getByRole("button", { name: "标记第 2 行" }));
    fireEvent.click(screen.getByLabelText("仅看标记"));
    expect(screen.queryByText("Alex joined the game")).not.toBeInTheDocument();
    expect(screen.getByText("Steve was slain by Zombie")).toBeInTheDocument();
    let exported: Blob | undefined;
    const create = vi.fn((blob: Blob) => {
      exported = blob;
      return "blob:test";
    });
    const OriginalURL = URL;
    vi.stubGlobal(
      "URL",
      class extends OriginalURL {
        static createObjectURL = create;
        static revokeObjectURL = vi.fn();
      },
    );
    const click = vi
      .spyOn(HTMLAnchorElement.prototype, "click")
      .mockImplementation(() => {});
    fireEvent.click(screen.getByRole("button", { name: "导出筛选结果 JSON" }));
    expect(click).toHaveBeenCalledOnce();
    expect(create).toHaveBeenCalledWith(expect.any(Blob));
    const text = await new Promise<string>((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => resolve(String(reader.result));
      reader.onerror = reject;
      reader.readAsText(exported!);
    });
    const data = JSON.parse(text);
    expect(data.events).toHaveLength(1);
    expect(data.events[0]).toMatchObject({
      line: 2,
      player: "Steve",
      bookmarked: true,
    });
    expect(text).not.toContain("Alex joined the game");
  });
  it("shows partial coverage and clears stale results when source changes", async () => {
    mount({ ...report, truncated: true });
    await analyze();
    expect(screen.getByRole("status")).toHaveTextContent("结果不完整");
    fireEvent.change(screen.getByLabelText("日志库"), {
      target: { value: "old.log.gz" },
    });
    expect(screen.queryByText("Alex joined the game")).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "导出筛选结果 JSON" }),
    ).not.toBeInTheDocument();
  });
  it("does not offer analysis for an empty log library", async () => {
    api.mockResolvedValue({ files: [], omitted: 0 });
    render(
      <LanguageProvider>
        <ActivityWorkbench
          id="empty"
          run={async (_, fn) => fn()}
          busy={new Set()}
        />
      </LanguageProvider>,
    );
    await screen.findByText("没有可读取的日志");
    expect(screen.getByRole("button", { name: "分析所选日志" })).toBeDisabled();
    expect(api).toHaveBeenCalledTimes(1);
  });
});
