import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import { cloudAccess, cloudNames, identityDraft, identityGist } from "./identity";

const settings: G.ProjectSettings = {
  auto_pull_request: true,
  remote_control: false,
  claude_ai_connectors: false,
  credit_people: true,
  credit_claude: true,
  publish_screenshots: false,
  package_caches: false,
  project_memory: true,
  retrospectives: true,
  id_tokens: true,
  env: {},
};

const ROLE = "arn:aws:iam::123456789012:role/fctl-web";
const PROVIDER = "projects/1/locations/global/workloadIdentityPools/fctl/providers/acme";

describe("a project's identity", () => {
  it("is drafted from its settings and back, what was typed trimmed", () => {
    const draft = identityDraft(settings);
    expect(draft).toEqual({
      id_tokens: true,
      aws_role: "",
      aws_region: "",
      aws_session_tags: false,
      gcp_provider: "",
      gcp_service_account: "",
      azure_tenant: "",
      azure_client: "",
    });
    expect(cloudAccess(draft)).toEqual({});
    const typed = {
      ...draft,
      aws_role: ` ${ROLE} `,
      aws_region: "eu-north-1 ",
      aws_session_tags: true,
      gcp_provider: PROVIDER,
      azure_tenant: " t ",
      azure_client: "c",
    };
    const access = cloudAccess(typed);
    expect(access).toEqual({
      aws: { role_arn: ROLE, region: "eu-north-1", session_tags: true },
      gcp: { provider: PROVIDER },
      azure: { tenant_id: "t", client_id: "c" },
    });
    expect(identityDraft({ ...settings, cloud_access: access })).toEqual({
      ...typed,
      aws_role: ROLE,
      aws_region: "eu-north-1",
      azure_tenant: "t",
    });
  });

  it("leaves out a cloud whose first field is empty, and what goes with it", () => {
    const draft = {
      ...identityDraft(settings),
      aws_region: "eu-north-1",
      aws_session_tags: true,
      gcp_service_account: "a@b.iam.gserviceaccount.com",
    };
    expect(cloudAccess(draft)).toEqual({});
    // Azure goes to the control plane with either id, which says what's missing.
    expect(cloudAccess({ ...draft, azure_client: "c" })).toEqual({ azure: { tenant_id: "", client_id: "c" } });
  });

  it("says in the list of pages whether agents mint tokens, and the clouds they reach", () => {
    expect(identityGist(settings)).toBe("Agents mint tokens");
    expect(identityGist({ ...settings, id_tokens: false })).toBe("Agents mint no tokens");
    const both = {
      ...settings,
      cloud_access: { aws: { role_arn: ROLE }, azure: { tenant_id: "t", client_id: "c" } },
    };
    expect(cloudNames(both)).toEqual(["AWS", "Azure"]);
    expect(identityGist(both)).toBe("Agents mint tokens · AWS, Azure");
  });
});
