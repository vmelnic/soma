# soma-project-s3

Self-contained S3 object storage project for SOMA MCP.

Send objects to S3 through the SOMA runtime via MCP `invoke_port` calls. Five capabilities: `put_object`, `get_object`, `delete_object`, `presign_url`, `list_objects`.

Pre-compiled artifacts included:
- `bin/soma` — runtime binary
- `packs/s3/manifest.json` — pack manifest
- `packs/s3/libsoma_port_s3.dylib` — S3 port library

## AWS Setup

### 1. Create an S3 Bucket

1. Go to https://s3.console.aws.amazon.com/s3/buckets
2. Click **Create bucket**
3. Enter a globally unique **Bucket name** (e.g. `my-soma-bucket`)
4. Select your **AWS Region** (e.g. `us-east-1`)
5. Leave "Block all public access" enabled (default)
6. Click **Create bucket**

### 2. Create IAM Credentials

1. Go to https://console.aws.amazon.com/iam/home#/users
2. Click **Create user**
3. Enter a **User name** (e.g. `soma-s3`), click **Next**
4. Select **Attach policies directly**
5. Search for `AmazonS3FullAccess` and check it (or create a custom policy scoped to your bucket)
6. Click **Next**, then **Create user**
7. Click the user name you just created to open its detail page
8. Go to the **Security credentials** tab
9. Scroll to **Access keys**, click **Create access key**
10. Select **Application running outside AWS**, click **Next**
11. Click **Create access key**
12. Copy the **Access key ID** and **Secret access key** — the secret is shown only once

### 3. Configure .env

```
AWS_ACCESS_KEY_ID=AKIA...your-key...
AWS_SECRET_ACCESS_KEY=...your-secret...
SOMA_S3_REGION=us-east-1
SOMA_S3_ENDPOINT=
SOMA_S3_DEFAULT_BUCKET=my-soma-bucket
```

Leave `SOMA_S3_ENDPOINT` empty for real AWS.

## Run

List the loaded S3 skills:

```bash
./scripts/list-skills.sh
```

Run the full smoke test (put, list, get, presign, delete):

```bash
./scripts/test-all.sh
```

## Node Client Commands

```bash
node mcp-client.mjs skills
node mcp-client.mjs put --key demo/hello.txt --file samples/hello.txt
node mcp-client.mjs list --prefix demo/
node mcp-client.mjs get --key demo/hello.txt --out samples/downloaded.txt
node mcp-client.mjs presign --key demo/hello.txt --expires-seconds 300
node mcp-client.mjs delete --key demo/hello.txt
node mcp-client.mjs smoke
```

## Run SOMA MCP Directly

```bash
./scripts/run-mcp.sh
```

Register `./scripts/run-mcp.sh` as the stdio MCP server command in Claude Code or any MCP client.
