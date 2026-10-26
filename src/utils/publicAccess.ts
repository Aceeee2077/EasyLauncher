import type { MessageKey } from "../i18n";
import type { PublicAccessPhase, PublicAccessProvider } from "../types";

export const providerKeys: Record<PublicAccessProvider, MessageKey> = {
  lan: "public.provider.lan",
  sakura: "public.provider.sakura",
  playit: "public.provider.playit",
  custom: "public.provider.custom",
};

export const phaseKey = (phase: PublicAccessPhase): MessageKey =>
  `public.phase.${phase}` as MessageKey;
