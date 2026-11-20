// A project's identity tokens and the clouds its agents reach with them (Settings → Identity): what
// its form edits, how what was typed becomes the settings the control plane takes, and what the list
// of Settings' pages says of it.

import type * as G from "@/gen";

/** What the Identity page edits, as typed: whether agents mint tokens, and each cloud's fields. */
export interface IdentityDraft {
  id_tokens: boolean;
  aws_role: string;
  aws_region: string;
  aws_session_tags: boolean;
  gcp_provider: string;
  gcp_service_account: string;
  azure_tenant: string;
  azure_client: string;
}

/** The draft of what a project's settings say now. */
export function identityDraft(s: G.ProjectSettings): IdentityDraft {
  const c = s.cloud_access ?? {};
  return {
    id_tokens: s.id_tokens,
    aws_role: c.aws?.role_arn ?? "",
    aws_region: c.aws?.region ?? "",
    aws_session_tags: c.aws?.session_tags ?? false,
    gcp_provider: c.gcp?.provider ?? "",
    gcp_service_account: c.gcp?.service_account ?? "",
    azure_tenant: c.azure?.tenant_id ?? "",
    azure_client: c.azure?.client_id ?? "",
  };
}

/**
 * The clouds a draft reaches: each whose first field has something in it, trimmed; the control plane
 * checks the rest. Azure takes both of its ids, and gets its error from the control plane when one
 * is missing.
 */
export function cloudAccess(d: IdentityDraft): G.CloudAccess {
  const t = (v: string) => v.trim();
  const out: G.CloudAccess = {};
  if (t(d.aws_role))
    out.aws = {
      role_arn: t(d.aws_role),
      ...(t(d.aws_region) ? { region: t(d.aws_region) } : {}),
      ...(d.aws_session_tags ? { session_tags: true } : {}),
    };
  if (t(d.gcp_provider))
    out.gcp = {
      provider: t(d.gcp_provider),
      ...(t(d.gcp_service_account) ? { service_account: t(d.gcp_service_account) } : {}),
    };
  if (t(d.azure_tenant) || t(d.azure_client))
    out.azure = { tenant_id: t(d.azure_tenant), client_id: t(d.azure_client) };
  return out;
}

/** The clouds a project's agents reach, by name, in the page's order. */
export function cloudNames(s: G.ProjectSettings): string[] {
  const c = s.cloud_access ?? {};
  return [c.aws && "AWS", c.gcp && "Google Cloud", c.azure && "Azure"].filter((n): n is string => !!n);
}

/** What the list of Settings' pages says: "Agents mint tokens · AWS, Azure", "Agents mint none". */
export function identityGist(s: G.ProjectSettings): string {
  if (!s.id_tokens) return "Agents mint no tokens";
  const clouds = cloudNames(s);
  return clouds.length ? `Agents mint tokens · ${clouds.join(", ")}` : "Agents mint tokens";
}

/** The fields the page shows the control plane's errors under. */
export const IDENTITY_FIELDS = {
  aws_role: "settings.cloud_access.aws.role_arn",
  aws_region: "settings.cloud_access.aws.region",
  gcp_provider: "settings.cloud_access.gcp.provider",
  gcp_service_account: "settings.cloud_access.gcp.service_account",
  azure_tenant: "settings.cloud_access.azure.tenant_id",
  azure_client: "settings.cloud_access.azure.client_id",
} as const;
