# Agents' work in your data warehouse

At the end of this guide, every event of the workspace `acme` that a
webhook posts lands as a row in BigQuery or Snowflake within a minute:
sessions starting, finishing a turn, ready for review and failing,
agents' pull requests opening and merging, and reviews with their
findings. Your dashboards then count sessions a day by project and how
they started, the time from a task to pull requests ready for review,
merged pull requests, review findings by repository, and tokens and
changed lines.

A small receiver of yours takes the webhook: a Cloud Run function that
writes to BigQuery, or a Lambda function that writes to S3, which
Snowpipe loads into Snowflake. An admin of `acme` adds the webhook and
makes the token that reads sessions' numbers; someone who can deploy in
your Google Cloud project, or your AWS and Snowflake accounts, does the
rest, in about an hour.

```text
fctl ──webhook──▶ the receiver ──a row──▶ BigQuery
                  │                       or S3 ──Snowpipe──▶ Snowflake
                  └─ session.finished: GET /api/v1/sessions/{id} (stats)
```

The diagram: fctl posts each event to the receiver, which checks it's
fctl's, adds a finished session's numbers from fctl's API, and writes one
row per event to the warehouse, directly for BigQuery or through S3 and
Snowpipe for Snowflake.

## What you need

- An admin of `acme`, for the webhook and the token.
- A Google Cloud project with billing (BigQuery takes no streaming
  inserts on its free tier), or an AWS account and a Snowflake account
  where you may create storage integrations (`ACCOUNTADMIN`).
- Node.js 24 and npm, to package the receiver.
- A receiver that fctl's webhooks reach: Cloud Run's and Lambda's
  `https://` addresses are on the internet, which every instance's
  webhooks reach. Where the receiver reads sessions' numbers, it reaches
  your instance too: from the internet for app.fctl.sh, or through your
  network for an instance of your own.

## Your values

```sh
export FCTL_SERVER=https://fctl.acme.dev
export FCTL_WORKSPACE=acme
```

## 1. Make the token that reads sessions

Events carry what happened, not what it cost: tokens and changed lines
are in each session's `stats`, which the receiver reads with a token
that only reads. An admin of `acme` makes it, so that it sees every
project's sessions:

```sh
fctl token "Warehouse" --access read -w "$FCTL_WORKSPACE" --days 365 > read-token
```

It writes the token (`fctl_pat_…`) to `read-token`, once. It reads what
its admin reads in `acme`, starts and changes nothing, and stops working
if they leave the workspace: then another admin makes a new one.

**Check:** `FCTL_TOKEN=$(cat read-token) fctl whoami` says it reads, in
`acme`.

## 2. Write the receiver

`row.mjs` turns an event into the row both warehouses store: the facts
queries group by, in columns, and the whole event as JSON.

```js
// One row per event: the facts queries group by, and the whole event.
export function row(event, stats) {
  const { session, review } = event.data ?? {};
  const pull = event.data?.pull_request ?? review?.pull_request;
  return {
    id: event.id, // the same in every attempt and resend: the views keep one row per id
    type: event.type,
    occurred_at: event.timestamp,
    test: event.test === true,
    workspace: event.workspace?.slug ?? null,
    project: event.project?.slug ?? null,
    session_id: session?.id ?? null,
    session_ref: session?.ref ?? null,
    started_by: session?.started_by ?? null,
    person_id: session?.created_by?.id ?? null,
    status: session?.status ?? null,
    model: session?.model ?? null,
    turns: session?.turns ?? null,
    repo: pull?.repo ?? null,
    pull_number: pull?.number ?? null,
    findings: review?.findings ?? null,
    input_tokens: stats?.input_tokens ?? null,
    output_tokens: stats?.output_tokens ?? null,
    additions: stats?.additions ?? null,
    deletions: stats?.deletions ?? null,
    payload: event,
    received_at: new Date().toISOString(),
  };
}

// A finished session's numbers so far: tokens, tool calls, changed files and lines.
export async function statsOf(event) {
  if (event.type !== "session.finished" || event.test) return undefined;
  try {
    const res = await fetch(`${process.env.FCTL_SERVER}/api/v1/sessions/${event.data.session.id}`, {
      headers: { Authorization: `Bearer ${process.env.FCTL_TOKEN}` },
      signal: AbortSignal.timeout(4000),
    });
    return res.ok ? (await res.json()).stats : undefined;
  } catch {
    return undefined; // the row goes in without them
  }
}
```

Each receiver answers fctl's challenge before anything else, as the
webhook's secret reaches you only after it: then it checks each request
with the secret, writes the row, and only then answers, within the 10
seconds fctl waits. A request it fails answers with an error, and fctl
sends it again ([When a request fails](automation.md#when-a-request-fails)).

## 3a. Into BigQuery

```sh
export PROJECT=acme-data REGION=europe-west1
gcloud config set project "$PROJECT"
gcloud services enable run.googleapis.com cloudbuild.googleapis.com artifactregistry.googleapis.com \
  bigquery.googleapis.com secretmanager.googleapis.com
```

### The table

```sh
bq mk --dataset --location=EU "$PROJECT:fctl"
```

Then, in the BigQuery console or with `bq query --use_legacy_sql=false`:

```sql
CREATE TABLE fctl.events_raw (
  id STRING NOT NULL, type STRING NOT NULL, occurred_at TIMESTAMP NOT NULL, test BOOL NOT NULL,
  workspace STRING, project STRING, session_id STRING, session_ref STRING, started_by STRING,
  person_id STRING, status STRING, model STRING, turns INT64, repo STRING, pull_number INT64,
  findings INT64, input_tokens INT64, output_tokens INT64, additions INT64, deletions INT64,
  payload JSON, received_at TIMESTAMP NOT NULL)
PARTITION BY DATE(occurred_at)
CLUSTER BY project, type;

-- One row per event, without fctl's tests: what queries read.
CREATE VIEW fctl.events AS
SELECT * FROM fctl.events_raw
WHERE NOT test
QUALIFY ROW_NUMBER() OVER (PARTITION BY id ORDER BY received_at) = 1;
```

### The function

In a directory of its own, with `row.mjs` beside it:

```sh
npm init -y && npm pkg set type=module
npm install @google-cloud/functions-framework @google-cloud/bigquery standardwebhooks
```

`index.js`:

```js
import { http } from "@google-cloud/functions-framework";
import { BigQuery } from "@google-cloud/bigquery";
import { Webhook } from "standardwebhooks";
import { row, statsOf } from "./row.mjs";

const table = new BigQuery().dataset("fctl").table("events_raw");

http("fctl", async (req, res) => {
  let event;
  try {
    if (req.body?.type === "url_verification") return res.type("text").send(req.body.challenge);
    event = new Webhook(process.env.FCTL_WEBHOOK_SECRET).verify(req.rawBody, req.headers);
  } catch {
    return res.status(400).end();
  }
  const r = row(event, await statsOf(event));
  // insertId drops a repeat that comes within a minute or so; the view, any later one.
  await table.insert([{ insertId: r.id, json: { ...r, payload: JSON.stringify(r.payload) } }], { raw: true });
  res.status(204).end();
});
```

It runs as a service account of its own, which writes to the dataset
and reads the token:

```sh
gcloud iam service-accounts create fctl-warehouse
bq query --use_legacy_sql=false \
  "GRANT \`roles/bigquery.dataEditor\` ON SCHEMA \`$PROJECT\`.fctl TO 'serviceAccount:fctl-warehouse@$PROJECT.iam.gserviceaccount.com'"
gcloud secrets create fctl-read-token --data-file=read-token
gcloud secrets add-iam-policy-binding fctl-read-token --role=roles/secretmanager.secretAccessor \
  --member="serviceAccount:fctl-warehouse@$PROJECT.iam.gserviceaccount.com"
gcloud run deploy fctl-warehouse --source . --function fctl --base-image nodejs24 --region "$REGION" \
  --no-invoker-iam-check --service-account "fctl-warehouse@$PROJECT.iam.gserviceaccount.com" \
  --set-env-vars "FCTL_SERVER=$FCTL_SERVER" --set-secrets FCTL_TOKEN=fctl-read-token:latest
```

It prints the function's address (`https://fctl-warehouse-….run.app`):
keep it as `RECEIVER`, and delete `read-token` from your machine.
`--no-invoker-iam-check` lets anyone call it, as fctl must: the
signature check is what keeps others out.

### The webhook

An admin of `acme` adds it, for every project:

```sh
fctl webhooks add "$RECEIVER" -w "$FCTL_WORKSPACE" --name "Warehouse" \
  --events 'session.*,pull_request.*,review.finished' > webhook-secret
```

The function answers fctl's challenge, and fctl writes the webhook's
secret (`whsec_…`) to `webhook-secret`, once. Give it to the function:

```sh
gcloud secrets create fctl-webhook-secret --data-file=webhook-secret
gcloud secrets add-iam-policy-binding fctl-webhook-secret --role=roles/secretmanager.secretAccessor \
  --member="serviceAccount:fctl-warehouse@$PROJECT.iam.gserviceaccount.com"
gcloud run services update fctl-warehouse --region "$REGION" --update-secrets FCTL_WEBHOOK_SECRET=fctl-webhook-secret:latest
```

**Check:** send a test, then find its row:

```sh
fctl webhooks test "Warehouse" -w "$FCTL_WORKSPACE" --event session.finished   # answers 204
bq query --use_legacy_sql=false 'SELECT type, occurred_at FROM fctl.events_raw WHERE test ORDER BY received_at DESC LIMIT 1'
```

The test's row is there, marked `test`, which the view leaves out. The
next session anyone starts in `acme` adds rows to `fctl.events`.

## 3b. Into Snowflake

The Lambda function writes each event to S3 as a file named by its id,
and Snowpipe loads new files into Snowflake within a minute or so. A
repeat of an event writes the same file again, which Snowpipe skips for
14 days; the view drops any later one.

```sh
export AWS_REGION=eu-north-1 BUCKET=acme-fctl-events
export ACCOUNT="$(aws sts get-caller-identity --query Account --output text)"
aws s3api create-bucket --bucket "$BUCKET" --create-bucket-configuration LocationConstraint="$AWS_REGION"
```

### The function

In a directory of its own, with `row.mjs` beside it:

```sh
npm init -y && npm pkg set type=module
npm install standardwebhooks @aws-sdk/client-s3
```

`index.mjs`:

```js
import { PutObjectCommand, S3Client } from "@aws-sdk/client-s3";
import { Webhook } from "standardwebhooks";
import { row, statsOf } from "./row.mjs";

const s3 = new S3Client();

export async function handler(request) {
  const body = request.isBase64Encoded ? Buffer.from(request.body, "base64").toString("utf8") : (request.body ?? "");
  let event;
  try {
    const sent = JSON.parse(body);
    if (sent.type === "url_verification") {
      return { statusCode: 200, headers: { "content-type": "text/plain" }, body: sent.challenge };
    }
    event = new Webhook(process.env.FCTL_WEBHOOK_SECRET).verify(body, request.headers);
  } catch {
    return { statusCode: 400 };
  }
  const day = event.timestamp.slice(0, 10).replaceAll("-", "/"); // when it happened: the same in every attempt
  await s3.send(
    new PutObjectCommand({
      Bucket: process.env.BUCKET,
      Key: `fctl/events/${day}/${event.id}.json`,
      Body: JSON.stringify(row(event, await statsOf(event))),
      ContentType: "application/json",
    }),
  );
  return { statusCode: 204 };
}
```

It runs with a role that writes the events' files and nothing else, and
answers at an address of its own, a function URL, which anyone may call:
the signature check is what keeps others out.

```sh
zip -qr function.zip index.mjs row.mjs package.json node_modules
aws iam create-role --role-name fctl-warehouse --assume-role-policy-document \
  '{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Principal":{"Service":"lambda.amazonaws.com"},"Action":"sts:AssumeRole"}]}'
aws iam attach-role-policy --role-name fctl-warehouse \
  --policy-arn arn:aws:iam::aws:policy/service-role/AWSLambdaBasicExecutionRole
aws iam put-role-policy --role-name fctl-warehouse --policy-name events --policy-document \
  "{\"Version\":\"2012-10-17\",\"Statement\":[{\"Effect\":\"Allow\",\"Action\":\"s3:PutObject\",\"Resource\":\"arn:aws:s3:::$BUCKET/fctl/events/*\"}]}"
aws lambda create-function --function-name fctl-warehouse --runtime nodejs24.x --handler index.handler \
  --role "arn:aws:iam::$ACCOUNT:role/fctl-warehouse" --zip-file fileb://function.zip --timeout 10 \
  --environment "Variables={BUCKET=$BUCKET,FCTL_SERVER=$FCTL_SERVER,FCTL_TOKEN=$(cat read-token)}"
aws lambda create-function-url-config --function-name fctl-warehouse --auth-type NONE
aws lambda add-permission --function-name fctl-warehouse --statement-id url --principal '*' \
  --action lambda:InvokeFunctionUrl --function-url-auth-type NONE
aws lambda add-permission --function-name fctl-warehouse --statement-id url-invoke --principal '*' \
  --action lambda:InvokeFunction --invoked-via-function-url
```

`create-function-url-config` prints the address
(`https://….lambda-url.eu-north-1.on.aws/`): keep it as `RECEIVER`. A
new role can take a few seconds to reach Lambda: if `create-function`
says it can't assume the role, run it again.

### The webhook

An admin of `acme` adds it, for every project:

```sh
fctl webhooks add "$RECEIVER" -w "$FCTL_WORKSPACE" --name "Warehouse" \
  --events 'session.*,pull_request.*,review.finished' > webhook-secret
aws lambda update-function-configuration --function-name fctl-warehouse --environment \
  "Variables={BUCKET=$BUCKET,FCTL_SERVER=$FCTL_SERVER,FCTL_TOKEN=$(cat read-token),FCTL_WEBHOOK_SECRET=$(cat webhook-secret)}"
```

Then delete `read-token` and `webhook-secret` from your machine. The
function's settings hold both: whoever reads its configuration reads
them, so keep that to the people who run it, or have the function read
them from Secrets Manager.

### The table, and Snowpipe

In Snowflake, as `ACCOUNTADMIN` or a role that may create integrations,
in the database of your choice:

```sql
CREATE STORAGE INTEGRATION fctl_events TYPE = EXTERNAL_STAGE STORAGE_PROVIDER = 'S3' ENABLED = TRUE
  STORAGE_AWS_ROLE_ARN = 'arn:aws:iam::123456789012:role/snowflake-fctl-events'
  STORAGE_ALLOWED_LOCATIONS = ('s3://acme-fctl-events/fctl/events/');
DESC INTEGRATION fctl_events;
```

`DESC INTEGRATION` gives `STORAGE_AWS_IAM_USER_ARN` and
`STORAGE_AWS_EXTERNAL_ID`, which Snowflake reads the bucket with. Make
the role it names, with those two in its trust policy:

```sh
aws iam create-role --role-name snowflake-fctl-events --assume-role-policy-document '{
  "Version": "2012-10-17",
  "Statement": [{ "Effect": "Allow", "Principal": { "AWS": "<STORAGE_AWS_IAM_USER_ARN>" }, "Action": "sts:AssumeRole",
    "Condition": { "StringEquals": { "sts:ExternalId": "<STORAGE_AWS_EXTERNAL_ID>" } } }]
}'
aws iam put-role-policy --role-name snowflake-fctl-events --policy-name read --policy-document "{
  \"Version\": \"2012-10-17\",
  \"Statement\": [
    { \"Effect\": \"Allow\", \"Action\": [\"s3:GetObject\", \"s3:GetObjectVersion\"], \"Resource\": \"arn:aws:s3:::$BUCKET/fctl/events/*\" },
    { \"Effect\": \"Allow\", \"Action\": [\"s3:ListBucket\", \"s3:GetBucketLocation\"], \"Resource\": \"arn:aws:s3:::$BUCKET\",
      \"Condition\": { \"StringLike\": { \"s3:prefix\": [\"fctl/events/*\"] } } }
  ]
}"
```

Then the table, the view and the pipe, in the schema `fctl`:

```sql
CREATE SCHEMA IF NOT EXISTS fctl;
CREATE STAGE fctl.events_stage URL = 's3://acme-fctl-events/fctl/events/' STORAGE_INTEGRATION = fctl_events;

CREATE TABLE fctl.events_raw (
  id STRING, type STRING, occurred_at TIMESTAMP_TZ, test BOOLEAN,
  workspace STRING, project STRING, session_id STRING, session_ref STRING, started_by STRING,
  person_id STRING, status STRING, model STRING, turns INT, repo STRING, pull_number INT,
  findings INT, input_tokens INT, output_tokens INT, additions INT, deletions INT,
  payload VARIANT, received_at TIMESTAMP_NTZ);

-- One row per event, without fctl's tests: what queries read.
CREATE VIEW fctl.events AS
SELECT * FROM fctl.events_raw
WHERE NOT test
QUALIFY ROW_NUMBER() OVER (PARTITION BY id ORDER BY received_at) = 1;

CREATE PIPE fctl.events_pipe AUTO_INGEST = TRUE AS
  COPY INTO fctl.events_raw
  FROM (SELECT $1:id::string, $1:type::string, TO_TIMESTAMP_TZ(REPLACE($1:occurred_at::string, 'Z', '+00:00')),
               $1:test::boolean, $1:workspace::string, $1:project::string, $1:session_id::string,
               $1:session_ref::string, $1:started_by::string, $1:person_id::string, $1:status::string,
               $1:model::string, $1:turns::int, $1:repo::string, $1:pull_number::int, $1:findings::int,
               $1:input_tokens::int, $1:output_tokens::int, $1:additions::int, $1:deletions::int,
               $1:payload, METADATA$FILE_LAST_MODIFIED
        FROM @fctl.events_stage)
  FILE_FORMAT = (TYPE = 'JSON');
SHOW PIPES LIKE 'events_pipe' IN SCHEMA fctl;
```

`SHOW PIPES` gives the pipe's `notification_channel`, a queue that S3
tells of new files:

```sh
aws s3api put-bucket-notification-configuration --bucket "$BUCKET" --notification-configuration '{
  "QueueConfigurations": [{ "QueueArn": "<notification_channel>", "Events": ["s3:ObjectCreated:*"],
    "Filter": { "Key": { "FilterRules": [{ "Name": "prefix", "Value": "fctl/events/" }] } } }]
}'
```

This replaces the bucket's other notifications, which a bucket of its
own doesn't have.

**Check:** send a test, then find its row a minute or two later:

```sh
fctl webhooks test "Warehouse" -w "$FCTL_WORKSPACE" --event session.finished   # answers 204
```

```sql
SELECT SYSTEM$PIPE_STATUS('fctl.events_pipe');   -- "executionState":"RUNNING"
SELECT type, occurred_at FROM fctl.events_raw WHERE test ORDER BY received_at DESC LIMIT 1;
```

The test's row is there, marked `test`, which the view leaves out.

## 4. Ask it things

The same queries run in both warehouses, on the view, unless a second
one follows for Snowflake.

Sessions a day, by project and by how they started (`person`,
`schedule`, `autopilot`, `ticket` or `service_account`):

```sql
SELECT DATE(occurred_at) AS on_day, project, started_by, COUNT(*) AS sessions
FROM fctl.events
WHERE type = 'session.started'
GROUP BY on_day, project, started_by
ORDER BY on_day DESC, sessions DESC;
```

The time from a session's task to its first pull requests ready for
review (checks passed, no review agent at work), in the last 30 days:

```sql
-- BigQuery
WITH started AS (
  SELECT session_id, ANY_VALUE(project) AS project, MIN(occurred_at) AS started_at
  FROM fctl.events WHERE type = 'session.started' GROUP BY session_id),
ready AS (
  SELECT session_id, MIN(occurred_at) AS ready_at
  FROM fctl.events WHERE type = 'session.ready_for_review' GROUP BY session_id)
SELECT project, COUNT(*) AS sessions,
  APPROX_QUANTILES(TIMESTAMP_DIFF(ready_at, started_at, MINUTE), 2)[OFFSET(1)] AS median_minutes
FROM started JOIN ready USING (session_id)
WHERE started_at >= TIMESTAMP_SUB(CURRENT_TIMESTAMP(), INTERVAL 30 DAY)
GROUP BY project
ORDER BY sessions DESC;
```

```sql
-- Snowflake
WITH started AS (
  SELECT session_id, ANY_VALUE(project) AS project, MIN(occurred_at) AS started_at
  FROM fctl.events WHERE type = 'session.started' GROUP BY session_id),
ready AS (
  SELECT session_id, MIN(occurred_at) AS ready_at
  FROM fctl.events WHERE type = 'session.ready_for_review' GROUP BY session_id)
SELECT project, COUNT(*) AS sessions, MEDIAN(DATEDIFF('minute', started_at, ready_at)) AS median_minutes
FROM started JOIN ready USING (session_id)
WHERE started_at >= DATEADD('day', -30, CURRENT_TIMESTAMP())
GROUP BY project
ORDER BY sessions DESC;
```

Agents' pull requests merged a day, by project:

```sql
SELECT DATE(occurred_at) AS on_day, project, COUNT(*) AS merged
FROM fctl.events
WHERE type = 'pull_request.merged'
GROUP BY on_day, project
ORDER BY on_day DESC, merged DESC;
```

Review agents' findings, by repository:

```sql
SELECT repo, COUNT(*) AS reviews, SUM(findings) AS findings,
  SUM(CASE WHEN findings = 0 THEN 1 ELSE 0 END) AS reviews_without_findings
FROM fctl.events
WHERE type = 'review.finished'
GROUP BY repo
ORDER BY findings DESC;
```

Tokens and changed lines, by project, from each session's last finished
turn that has them, whose numbers count all its turns until then (a row
whose read of the session failed has none):

```sql
WITH latest AS (
  SELECT * FROM fctl.events
  WHERE type = 'session.finished' AND output_tokens IS NOT NULL
  QUALIFY ROW_NUMBER() OVER (PARTITION BY session_id ORDER BY occurred_at DESC) = 1)
SELECT project, COUNT(*) AS sessions, SUM(input_tokens) AS input_tokens, SUM(output_tokens) AS output_tokens,
  SUM(additions) AS additions, SUM(deletions) AS deletions
FROM latest
GROUP BY project
ORDER BY output_tokens DESC;
```

Anything else an event says is in `payload`: a session's title, branch
and schedule, a pull request's checks, a review's commit. BigQuery
reads it as `JSON_VALUE(payload, '$.data.session.title')`, Snowflake as
`payload:data.session.title::string`
([What a request carries](automation.md#what-a-request-carries)).

## Sessions from before the webhook

The webhook posts what happens from now on. For the sessions before it,
read them from the API with the same token, a page of 200 at a time,
into a table of their own:

```sh
before=""
while :; do
  page=$(curl -sf -H "Authorization: Bearer $(cat read-token)" \
    "$FCTL_SERVER/api/v1/sessions?workspace=$FCTL_WORKSPACE&limit=200&archived=1&hidden=1${before:+&before=$before}")
  echo "$page" | jq -c '.sessions[]' >> sessions.jsonl
  before=$(echo "$page" | jq -r '.next_before // empty')
  [ -n "$before" ] || break
done
bq load --source_format=NEWLINE_DELIMITED_JSON --autodetect fctl.sessions sessions.jsonl   # or Snowflake's PUT and COPY INTO
```

Each line is a session as the API gives it
([Sessions](api.md#read-them)): its project, who started it and how,
its status, `created_at` and `ended_at` in milliseconds, its pull
requests and its `stats`.

## What goes wrong

| What you see | Why | What to do |
|---|---|---|
| `fctl webhooks add` says the address didn't answer its challenge | The function isn't deployed, or answers with an error | Its logs say why; Lambda's in CloudWatch, Cloud Run's in Cloud Logging |
| Tests answer `400` | The function doesn't have the webhook's secret yet, or an old one | Give it the secret again; after a rotation, both work for a day |
| Rows come, without tokens | The read token doesn't reach the session, or expired | `FCTL_TOKEN=… fctl show <session>`; make a new token |
| No rows in Snowflake, while S3 has files | The pipe doesn't hear of them | `SYSTEM$PIPE_STATUS`, the bucket's notification, and the role's trust policy |

The webhook's page in fctl, or `fctl webhooks deliveries "Warehouse" -w
acme --failed`, lists what went out and what the function answered.
Once it works again, `fctl webhooks resend "Warehouse" -w acme --failed
--since 1d` sends what failed.

## Limits

- **Events come out of order**, and some twice: group by `occurred_at`,
  never by when they arrived, and query the views, which keep one row
  per event.
- **A webhook drops events once 1,000 wait for it**, as when the
  function fails for a long while, and its page counts them. fctl keeps
  what went out for 30 days, to resend.
- **They say things about people.** Events name the person each session
  is for (`created_by`), and carry sessions' titles, the questions
  agents ask and up to 2,000 characters of each reply. Keep the tables
  to the people who may see those sessions in fctl, and delete rows as
  your retention rules say: what fctl deletes stays here.
