# infra

Romanesco's infrastructure as Terraform: AWS in eu-north-1 for the platform, its Postgres and its queues, and Cloudflare for DNS, the CDN and the WAF in front of the shop. Plus the workflows that plan and apply it.

## Layout

- `production/`, `staging/`: a root module for each environment, each with its state in S3.
- `modules/`: what both are made of: `network`, `database`, `platform` and `monitoring`.
- `cloudflare/`: DNS records and WAF rules for romanesco.farm.
- `docs/runbooks/`: what to do when an alarm pages you.

## Plans and applies

Each pull request gets a `terraform plan` for both environments from the `plan` workflow, as a comment. Merging to `main` applies them through the `apply` workflow, once someone approves the `production` environment. Nobody applies from a laptop.

```sh
terraform -chdir=staging init
terraform -chdir=staging plan -lock=false
```

`-lock=false`, because the keys that plan can read the state but not take its lock.
