import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { LanguageProvider } from "../i18n";
import { PlayitControl } from "./PlayitControl";
import type { PlayitInfo } from "../types";

const api = vi.hoisted(() => vi.fn());
vi.mock("../services/api", () => ({ desktop: true, call: api }));
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});
const base: PlayitInfo = {
  enabled: true,
  authorized: false,
  phase: "authorizing",
  address: null,
  claimUrl: "https://playit.gg/claim/0123456789",
  message: null,
};
function mount(info: PlayitInfo) {
  api.mockResolvedValue(info);
  const onAddress = vi.fn();
  render(
    <LanguageProvider>
      <PlayitControl
        id="server"
        run={async (_, fn) => fn()}
        busy={new Set()}
        onAddress={onAddress}
      />
    </LanguageProvider>,
  );
  return onAddress;
}
describe("automatic multiplayer", () => {
  it("offers authorization without path or port inputs", async () => {
    mount(base);
    fireEvent.click(
      await screen.findByRole("button", { name: "前往 playit 授权" }),
    );
    await waitFor(() => expect(api).toHaveBeenCalledWith("playit_open_claim"));
    expect(screen.queryByRole("textbox")).not.toBeInTheDocument();
  });
  it("does not share an allocated but unverified address", async () => {
    const onAddress = mount({
      ...base,
      authorized: true,
      claimUrl: null,
      phase: "connecting",
      address: "demo.ply.gg:25565",
    });
    expect(
      await screen.findByRole("button", { name: "复制地址" }),
    ).toBeDisabled();
    expect(onAddress).not.toHaveBeenCalledWith("demo.ply.gg:25565");
  });
  it("shares only a verified address and clears it on disable", async () => {
    const onAddress = mount({
      ...base,
      authorized: true,
      claimUrl: null,
      phase: "online",
      address: "demo.ply.gg:25565",
    });
    await waitFor(() =>
      expect(onAddress).toHaveBeenCalledWith("demo.ply.gg:25565"),
    );
    api.mockResolvedValue({ ...base, enabled: false, phase: "disabled" });
    fireEvent.click(screen.getByRole("button", { name: "关闭自动联机" }));
    await waitFor(() =>
      expect(api).toHaveBeenCalledWith("playit_enable", {
        id: "server",
        enabled: false,
      }),
    );
    await waitFor(() => expect(onAddress).toHaveBeenLastCalledWith(""));
  });
});
