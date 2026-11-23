# Runners in your cloud

What [Runners in your cloud](../../docs/runners-in-your-cloud.md) builds,
as files:

- `runner/`: the scripts and systemd units a runner's machine image
  holds: `provision.sh` makes the image, `fctl-enroll` enrolls the machine
  at its first boot, `fctl-scale-in-<cloud>` drains it before the cloud
  takes it, `fctl-retire-amazon-ebs` takes an idle machine out of its
  group, and `fctl-size-group-<cloud>` sizes the group to what its pool
  needs.
- `fctl-runner.pkr.hcl`: the Packer template that builds the image on
  AWS, Google Cloud and Azure from `runner/`.
- `terraform/aws`: a module that deploys a pool on AWS, image (EC2 Image
  Builder) and sizer (a Lambda function) included.
- `terraform/google` and `terraform/azure`: smaller examples, from an
  image Packer built.

The guide shows each file of `runner/` and the template as they are here:
`cargo test -p xtask docs` checks that, so change both together. Before
you push a change here:

```sh
shellcheck -s sh -S warning deploy/cloud/runner/provision.sh deploy/cloud/runner/fctl-[!f]*[!e] deploy/cloud/runner/fctl-*-*
for d in deploy/cloud/terraform/*/; do (cd "$d" && terraform fmt -check && terraform init -backend=false && terraform validate); done
```
