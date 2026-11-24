// What bringing a pool's machines online takes where they run (docs/runners-in-your-cloud.md): the
// steps of the new pool's setup, each with what to copy, every value of this instance and this pool
// filled in. AWS's comes from fctl's Terraform module, Google Cloud's and Azure's from its examples
// (deploy/cloud/terraform), Kubernetes' from KEDA's metrics-api scaler, which reads what the pool
// needs with its enrollment token.

import { enrollCommand } from "./enrollment";
import { installCommand } from "./install";

/** Where a pool's machines run. */
export type PoolPlace = "own" | "aws" | "google" | "azure" | "kubernetes";

export const PLACES: readonly { value: PoolPlace; label: string; description: string }[] = [
  {
    value: "own",
    label: "Your own machines",
    description: "Macs or Linux machines you set up: a boot script or a shell enrolls each.",
  },
  {
    value: "aws",
    label: "AWS",
    description:
      "An Auto Scaling group from fctl's Terraform module, which sizes it from what the pool needs.",
  },
  {
    value: "google",
    label: "Google Cloud",
    description: "A managed instance group from fctl's example, which its machines size.",
  },
  {
    value: "azure",
    label: "Azure",
    description: "A scale set from fctl's example, which its machines size.",
  },
  {
    value: "kubernetes",
    label: "Kubernetes",
    description: "A Deployment of hosts that KEDA scales from what the pool needs.",
  },
];

/** Machines that a cloud makes and destroys: their runners go once offline a while. */
export const autoscaled = (p: PoolPlace) => p !== "own";

/** Sessions each machine runs at once until someone says (`host.toml`'s capacity, the modules' default). */
export const DEFAULT_CAPACITY = 3;

/** What a pool's setup is made of: this instance, the pool, and its token. */
export interface SetupValues {
  /** The app's origin, which the machines enroll with: `https://app.fctl.sh`. */
  origin: string;
  /** The hostname the app runs at, whose site the installer comes from. */
  hostname: string;
  /** fctl's site beside the app, whose docs the steps link to: `https://fctl.sh`. */
  site: string;
  /** The pool's name. */
  name: string;
  /** Its enrollment token (`fctl_et_…`), shown once. */
  token: string;
  /** Sessions each machine runs at once. */
  capacity: number;
  /** The most runners the pool keeps at once, if it has a limit. */
  maxRunners?: number;
  /** The version the instance runs, which a cloud's image installs. */
  version: string;
}

/** A step of the setup: what it achieves, what to know, and what to copy. */
export interface SetupStep {
  title: string;
  body?: string;
  code?: string[];
  /** Where the guide says the rest. */
  link?: { href: string; label: string };
}

/** A name clouds and Terraform take, from the pool's: `AWS large` becomes `fctl-aws-large`. */
export function resourceName(pool: string): string {
  const slug = pool
    .toLowerCase()
    .normalize("NFKD")
    .replace(/[\u0300-\u036f]/g, "")
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 40)
    .replace(/-+$/, "");
  return slug ? `fctl-${slug}` : "fctl-runners";
}

/** The most machines a group keeps: a few below the pool's limit, where draining ones still count. */
export function groupMax(maxRunners: number | undefined): number | undefined {
  if (maxRunners === undefined) return undefined;
  return Math.max(1, maxRunners - Math.max(1, Math.ceil(maxRunners / 10)));
}

/** Lines of `key = value`, the values lined up, each with its comment. */
function hcl(lines: [string, string, string?][], indent = ""): string {
  const width = Math.max(...lines.map(([k]) => k.length));
  return lines
    .map(([k, value, comment]) => `${indent}${k.padEnd(width)} = ${value}${comment ? ` # ${comment}` : ""}`)
    .join("\n");
}

/** The steps that bring the pool's machines online where they run. */
export function setupSteps(place: PoolPlace, v: SetupValues): SetupStep[] {
  const name = resourceName(v.name);
  const max = groupMax(v.maxRunners);
  switch (place) {
    case "own":
      return [
        { title: "Install fctl on each machine", code: [installCommand(v.hostname)] },
        {
          title: "Enroll it, and start its host service",
          body: "As the user that service runs as. In a boot script (cloud-init's user data, a launch template) both run as they are; on Linux, where nobody logs in, run loginctl enable-linger for that user first.",
          code: [enrollCommand(v.origin, v.token, v.capacity === DEFAULT_CAPACITY ? undefined : v.capacity)],
        },
      ];
    case "aws":
      return [
        {
          title: "Keep the token in a file Terraform reads",
          body: "Out of git: Terraform puts it into Secrets Manager, and into its state, which you keep as safe as the token.",
          code: [`printf '%s' '${v.token}' > enroll-token`],
        },
        {
          title: "Add fctl's module to your Terraform",
          body: "It builds the image with EC2 Image Builder, makes the group with its drain hook, and a function that sizes it from what the pool needs every minute. Its machines retire once idle for 20 minutes. Your VPC and its private subnets, which reach the internet through a NAT gateway, are yours to fill in. Pin the source to a release by adding ?ref= and its tag.",
          link: {
            href: `${v.site}/docs/runners-in-your-cloud/#with-terraform`,
            label: "The module, in the guide",
          },
          code: [
            `module "${name.replace(/-/g, "_")}" {\n${hcl(
              [
                ["source", `"github.com/hyperstructure/fctl//deploy/cloud/terraform/aws"`],
                ["name", `"${name}"`],
                ["fctl_server", `"${v.origin}"`],
                ["fctl_version", `"${v.version}"`],
                ["enrollment_token", `file("enroll-token")`],
                ["capacity", String(v.capacity), "sessions each machine runs"],
                ...(max !== undefined
                  ? [
                      ["max_size", String(max), `a few below the pool's ${v.maxRunners} runners`] as [
                        string,
                        string,
                        string,
                      ],
                    ]
                  : []),
                ["vpc_id", `"vpc-…"`],
                ["subnet_ids", `["subnet-…", "subnet-…"]`],
              ],
              "  ",
            )}\n}`,
          ],
        },
        {
          title: "Apply it",
          body: "The first apply takes about 25 minutes, most of it the image. Machines then enroll here as the pool's runners within a minute of booting.",
          code: ["terraform init && terraform apply"],
        },
      ];
    case "google":
      return [
        {
          title: "Build the image",
          body: "With Packer, in fctl's deploy/cloud, from a project that may make images.",
          link: {
            href: `${v.site}/docs/runners-in-your-cloud/#6-on-google-cloud`,
            label: "Google Cloud, in the guide",
          },
          code: [
            `git clone https://github.com/hyperstructure/fctl && cd fctl/deploy/cloud\npacker init . && packer build -only='googlecompute.fctl-runner' -var "fctl_version=${v.version}" -var "gcp_project=YOUR_PROJECT" .`,
          ],
        },
        {
          title: "Fill in the example's values",
          body: "In deploy/cloud/terraform/google: a managed instance group whose machines size it every minute from what the pool needs, the token in Secret Manager.",
          code: [
            `cd terraform/google && cat > terraform.tfvars <<'EOF'\n${hcl([
              ["project", `"YOUR_PROJECT"`],
              ["subnetwork", `"YOUR_SUBNET"`, "with Cloud NAT"],
              ["name", `"${name}"`],
              ["fctl_server", `"${v.origin}"`],
              ["capacity", String(v.capacity), "sessions each machine runs"],
            ])}\nEOF`,
          ],
        },
        {
          title: "Apply it, with the token",
          body: "It goes into Secret Manager, and into Terraform's state, which you keep as safe as the token.",
          code: [`terraform init && TF_VAR_enrollment_token='${v.token}' terraform apply`],
        },
      ];
    case "azure":
      return [
        {
          title: "Build the image into a gallery",
          body: "With Packer, in fctl's deploy/cloud, once the resource group and its gallery are there.",
          link: { href: `${v.site}/docs/runners-in-your-cloud/#7-on-azure`, label: "Azure, in the guide" },
          code: [
            `git clone https://github.com/hyperstructure/fctl && cd fctl/deploy/cloud\npacker init . && packer build -only='azure-arm.fctl-runner' -var "fctl_version=${v.version}" \\\n  -var "azure_subscription=$SUBSCRIPTION" -var "azure_resource_group=$RG" -var "azure_location=$LOCATION" .`,
          ],
        },
        {
          title: "Fill in the example's values",
          body: "In deploy/cloud/terraform/azure: a scale set whose machines size it every minute from what the pool needs, the token in Key Vault.",
          code: [
            `cd terraform/azure && cat > terraform.tfvars <<'EOF'\n${hcl([
              ["subscription_id", `"YOUR_SUBSCRIPTION"`],
              ["subnet_id", `"YOUR_SUBNET_ID"`, "with a NAT gateway"],
              ["image_id", `"YOUR_IMAGE_VERSION_ID"`, "Packer's, in the gallery"],
              ["vault_name", `"${name.slice(0, 24)}"`, "unique in Azure"],
              ["admin_ssh_key", `"ssh-ed25519 …"`],
              ["name", `"${name}"`],
              ["fctl_server", `"${v.origin}"`],
              ["capacity", String(v.capacity), "sessions each machine runs"],
            ])}\nEOF`,
          ],
        },
        {
          title: "Apply it, with the token",
          body: "It goes into Key Vault, and into Terraform's state, which you keep as safe as the token.",
          code: [`terraform init && TF_VAR_enrollment_token='${v.token}' terraform apply`],
        },
      ];
    case "kubernetes":
      return [
        {
          title: "Keep the token in a secret",
          code: [
            `kubectl create namespace fctl\nkubectl -n fctl create secret generic fctl-enrollment-token --from-literal=token='${v.token}'`,
          ],
        },
        {
          title: "Run the host as a Deployment",
          body: `Named ${name}, each pod enrolling with the secret's token (FCTL_SERVER=${v.origin}, FCTL_ENROLL_TOKEN from the secret) and running the host with a driver it has there, a Docker daemon say. Its pods keep their deletion cost at their number of sessions, so that scaling down removes the idle ones first, and drain on preStop.`,
          link: { href: `${v.site}/docs/self-hosting/#hosts-in-kubernetes`, label: "The role and the hook" },
        },
        {
          title: "Let KEDA scale it from what the pool needs",
          body: "Its metrics-api scaler reads the pool's demand with the token, and sets the Deployment's replicas to how many runners would run everything.",
          code: [
            `kubectl apply -f - <<'EOF'\napiVersion: keda.sh/v1alpha1\nkind: TriggerAuthentication\nmetadata: { name: fctl-enrollment-token, namespace: fctl }\nspec:\n  secretTargetRef:\n    - { parameter: token, name: fctl-enrollment-token, key: token }\n---\napiVersion: keda.sh/v1alpha1\nkind: ScaledObject\nmetadata: { name: ${name}, namespace: fctl }\nspec:\n  scaleTargetRef: { name: ${name} }\n  minReplicaCount: 0\n  maxReplicaCount: ${max ?? 20}\n  triggers:\n    - type: metrics-api\n      metadata:\n        url: ${v.origin}/api/v1/runner/enroll/demand\n        valueLocation: wanted\n        targetValue: "1"\n        authMode: bearer\n      authenticationRef: { name: fctl-enrollment-token }\nEOF`,
          ],
        },
      ];
  }
}
