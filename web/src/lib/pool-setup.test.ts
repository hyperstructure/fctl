import { describe, expect, it } from "vitest";
import { groupMax, type PLACES, resourceName, type SetupValues, setupSteps } from "./pool-setup";

const v: SetupValues = {
  origin: "https://fctl.acme.dev",
  hostname: "fctl.acme.dev",
  site: "https://fctl.sh",
  name: "AWS large",
  token: "fctl_et_secret",
  capacity: 4,
  maxRunners: 20,
  version: "0.38.0",
};

const code = (place: (typeof PLACES)[number]["value"], values = v) =>
  setupSteps(place, values)
    .flatMap((s) => s.code ?? [])
    .join("\n");

describe("a pool's setup", () => {
  it("names what it makes after the pool, as clouds take names", () => {
    expect(resourceName("AWS large")).toBe("fctl-aws-large");
    expect(resourceName("  GPUs (eu-north-1)! ")).toBe("fctl-gpus-eu-north-1");
    expect(resourceName("Ünïcode pool")).toBe("fctl-unicode-pool");
    expect(resourceName("!!!")).toBe("fctl-runners");
    expect(groupMax(20)).toBe(18);
    expect(groupMax(3)).toBe(2);
    expect(groupMax(1)).toBe(1);
    expect(groupMax(undefined)).toBeUndefined();
  });

  it("enrolls one's own machines with the token, at their capacity", () => {
    const own = code("own");
    expect(own).toContain("curl -fsSL");
    expect(own).toContain(
      "FCTL_SERVER=https://fctl.acme.dev \\\nFCTL_ENROLL_TOKEN=fctl_et_secret \\\nfctl host install --capacity 4",
    );
    expect(code("own", { ...v, capacity: 3 })).toMatch(/fctl host install$/m);
  });

  it("fills in fctl's AWS module with every value it knows", () => {
    const aws = code("aws");
    expect(aws).toContain(`printf '%s' 'fctl_et_secret' > enroll-token`);
    expect(aws).toContain('module "fctl_aws_large" {');
    expect(aws).toContain(
      '  source           = "github.com/hyperstructure/fctl//deploy/cloud/terraform/aws"',
    );
    expect(aws).toContain('  fctl_server      = "https://fctl.acme.dev"');
    expect(aws).toContain('  fctl_version     = "0.38.0"');
    expect(aws).toContain('  enrollment_token = file("enroll-token")');
    expect(aws).toContain("  capacity         = 4 # sessions each machine runs");
    expect(aws).toContain("  max_size         = 18 # a few below the pool's 20 runners");
    expect(code("aws", { ...v, maxRunners: undefined })).not.toContain("max_size");
  });

  it("says where to find the version while the app doesn't know it", () => {
    const unknown = { ...v, version: undefined };
    expect(code("aws", unknown)).toContain(
      '  fctl_version     = "YOUR_FCTL_VERSION" # what the instance runs: its /api/v1/health says',
    );
    for (const place of ["google", "azure"] as const) {
      expect(code(place, unknown)).toContain('-var "fctl_version=YOUR_FCTL_VERSION"');
      expect(code(place, unknown)).not.toContain('fctl_version="');
    }
  });

  it("fills in the Google Cloud and Azure examples, the token kept out of their files", () => {
    const google = code("google");
    expect(google).toContain('-var "fctl_version=0.38.0"');
    expect(google).toContain('name        = "fctl-aws-large"');
    expect(google).toContain("TF_VAR_enrollment_token='fctl_et_secret' terraform apply");
    expect(google.split("EOF")[1]).not.toContain("fctl_et_secret");
    const azure = code("azure");
    expect(azure).toContain("azure-arm.fctl-runner");
    expect(azure).toContain('fctl_server     = "https://fctl.acme.dev"');
    expect(azure).toContain('vault_name      = "fctl-aws-large"');
  });

  it("makes the Azure image's gallery first, apart from the scale set's group", () => {
    const azure = code("azure");
    // The gallery, its group and its image definition, which Packer's build needs.
    expect(azure).toContain("RG=fctl-aws-large-images");
    expect(azure).toContain('az group create -n "$RG" -l "$LOCATION"');
    expect(azure).toContain('az sig create -g "$RG" --gallery-name fctl_aws_large');
    expect(azure).toContain("--gallery-image-definition fctl-runner");
    expect(azure).toContain('-var "azure_gallery=fctl_aws_large"');
    // The tfvars take the shell's values, the image from that gallery, and a group of the scale set's own.
    expect(azure).toContain("cat > terraform.tfvars <<EOF\n");
    expect(azure).toContain('subscription_id = "$SUBSCRIPTION"');
    expect(azure).toContain('location        = "$LOCATION"');
    expect(azure).toContain('resource_group  = "fctl-aws-large" # the scale set\'s, which it makes');
    expect(azure).toContain(
      '"/subscriptions/$SUBSCRIPTION/resourceGroups/$RG/providers/Microsoft.Compute/galleries/fctl_aws_large/images/fctl-runner/versions/0.38.0"',
    );
    // A Key Vault's name ends with a letter or a digit, within 24 characters.
    expect(code("azure", { ...v, name: "Build fleet in westeurope" })).toContain(
      'vault_name      = "fctl-build-fleet-in-west"',
    );
    expect(code("azure", { ...v, name: "Build fleet in abc def" })).toContain(
      'vault_name      = "fctl-build-fleet-in-abc"',
    );
  });

  it("scales a Deployment with KEDA from the pool's demand", () => {
    const k8s = code("kubernetes");
    expect(k8s).toContain(
      "create secret generic fctl-enrollment-token --from-literal=token='fctl_et_secret'",
    );
    expect(k8s).toContain("url: https://fctl.acme.dev/api/v1/runner/enroll/demand");
    expect(k8s).toContain("valueLocation: wanted");
    expect(k8s).toContain("authMode: bearer");
    expect(k8s).toContain("scaleTargetRef: { name: fctl-aws-large }");
    expect(k8s).toContain("maxReplicaCount: 18");
    // Every step that leaves something to the guide links to it.
    for (const place of ["aws", "google", "azure", "kubernetes"] as const)
      expect(setupSteps(place, v).some((s) => s.link?.href.startsWith("https://fctl.sh/docs/"))).toBe(true);
  });
});
