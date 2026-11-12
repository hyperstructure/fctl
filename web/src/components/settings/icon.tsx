// Each of Settings' pages has an icon: in its navigation, in its list on a phone, and in the
// command palette, which finds the pages too.

import {
  Bell,
  CircleUserRound,
  Cpu,
  Database,
  KeyRound,
  LayoutGrid,
  type LucideIcon,
  Mail,
  ScrollText,
  ShieldCheck,
  SunMoon,
} from "lucide-react";
import { LinearIcon } from "@/components/ui";
import { cn } from "@/lib/cn";
import type { SettingsTab } from "@/lib/settings";

const ICONS: Record<Exclude<SettingsTab, "linear">, LucideIcon> = {
  profile: CircleUserRound,
  emails: Mail,
  workspaces: LayoutGrid,
  claude: Cpu,
  notifications: Bell,
  appearance: SunMoon,
  log: ScrollText,
  security: ShieldCheck,
  tokens: KeyRound,
  data: Database,
};

export function SettingsIcon({ tab, className }: { tab: SettingsTab; className?: string }) {
  // Linear's mark is filled: a little smaller, it weighs as much as the outlined icons.
  if (tab === "linear") return <LinearIcon className={cn(className, "scale-[0.85]")} />;
  const Icon = ICONS[tab];
  return <Icon className={className} aria-hidden="true" />;
}
