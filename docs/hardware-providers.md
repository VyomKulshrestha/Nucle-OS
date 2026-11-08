# Hardware provider CLI

`nucle hardware export` can submit to `twist`, `idt`, `illumina`, and
`nanopore` without placing secrets on the command line or in a config file.
The config names environment variables that hold credentials; the generated
job file contains only provider job identifiers and request metadata.

Vendor submission is deliberately asynchronous:

```bash
nucle hardware export archive.nsl \
  --provider twist \
  --config twist.json \
  --job-file twist-job.jsonl \
  --confirm

nucle hardware status twist-job.jsonl --config twist.json
```

Inspect the provider boundary without credentials or network access:

```bash
nucle hardware capabilities
nucle hardware capabilities idt
nucle --json hardware capabilities
```

This reports the adapter's static request/profile contract, resumability, and
network requirement. It is not live instrument discovery and does not claim a
configured account or instrument is currently available.

The source program must still contain its language-level `confirm hardware`
or `confirm physical_key`, and the operator must still pass `--confirm` for
any cost-bearing or destructive batch. A configured `confirm-users` policy is
also checked before any provider request. Config loading does not bypass or
replace any of these gates.

## Safety behavior

- Vendor configs are strict JSON: missing, unknown, or provider-mismatched
  fields are rejected.
- Secrets are loaded only from the named environment variables. Missing or
  empty variables stop the command before a job file or network request is
  created.
- Credential-bearing HTTP calls require HTTPS. Plain HTTP is accepted only
  for an explicit `localhost`, `127.0.0.1`, or `[::1]` test server.
- Vendor endpoint paths must remain on the configured host. Status templates
  contain exactly one `{id}` placeholder, and returned job IDs are validated
  and percent-encoded before use in a URL.
- Vendor source files are type- and profile-scoped: Twist accepts only `Twist`
  synthesis, IDT accepts only `IDT` synthesis, Illumina accepts only `Illumina`
  sequencing, and Nanopore accepts only `Nanopore` sequencing. Empty, mixed, destructive,
  QC, or recovery batches are rejected before job-file creation or network
  access instead of being silently coerced into a vendor payload.
- `--job-file` uses append-only JSON Lines and refuses to overwrite an existing
  file. Each accepted job token is flushed and synced before the next source is
  submitted, so an earlier accepted job remains resumable if a later source
  fails.
- A successful vendor export means `Submitted`, not `Complete`. Use
  `hardware status` until the provider reports a terminal state. A polling or
  transport error exits non-zero but does not remove the job file; retrying the
  status command is safe.
- HTTP and MinKNOW connection/RPC calls have bounded five-second timeouts.
- Library callers can use `JobHandle::wait_timeout` or
  `Provider::execute_batch_timeout` to bound a complete job wait. A local
  timeout does not cancel the remote job; its resume token remains usable.

No live credentials are needed by the test suite. Twist, IDT, and Illumina CLI
tests use loopback HTTP stand-ins. Nanopore uses a generated test certificate
and a loopback TLS gRPC MinKNOW stand-in to exercise discovery, submission,
resume, and status polling without an instrument; CLI tests separately verify
credential/config refusal.

## Order sheets

The API adapters below need an account's private endpoints, and no
NucleScript program has yet been sent through one to a real synthesizer.
`nucle hardware order` produces what a person can submit through a
vendor's ordinary ordering page instead:

```bash
nucle hardware order archive.nsl --vendor twist --out order.fasta
nucle hardware order archive.nsl --vendor idt --out order.csv
```

It encodes every top-level `store` and `pipeline` that writes for real
(simulated stores are left out), exactly as `nucle run` would but with no
noise, into a fresh pool, and writes one oligo per strand: the archive
header's strands first (`header_0`, ...), then each file's data and parity
strands with their primers (`archive_records.csv_d0`, `..._p0`). Because
the header is included, `nucle bootstrap inspect order.fasta` gives back
the file table from the sheet alone. Stores inside functions are not
collected, the same as for `hardware export`.

The sheet is checked against the limits each vendor publishes, and none is
written if the vendor would refuse it:

| Vendor | File | Oligo length | Pool size |
|---|---|---|---|
| Twist | `.fasta` or `.csv` (`Name,Sequence`) | 20-350 nt | not checked |
| IDT oPools | `.csv` (`Name,Sequence`), pasted into the Excel template IDT's page provides | 40-350 nt | 2-384 oligos at 50 pmol, 10-2,000 at 10 pmol, 100-20,000 at 1 pmol |

Every sequence must be plain A/C/G/T and every name unique. For IDT the
command lists the scales that take a pool of that size. The IDT figures are
from its oPools entry page, and still matched it on 8 November 2026. The
page takes Excel files only (`.xlsx`, `.xls`) and, without an IDT account,
shows neither a template nor its column names, hence the CSV to paste. The Twist
figures are its oligo-pool requirements as university core facilities pass
them on (a sequence file with one oligo per line). Nothing is ordered;
check the sheet against the vendor's current page before uploading it.

## Twist

```json
{
  "provider": "twist",
  "base_url": "https://your-account-api.example",
  "order_path": "/orders",
  "status_path_template": "/orders/{id}",
  "email_env": "NUCLE_TWIST_EMAIL",
  "api_token_env": "NUCLE_TWIST_API_TOKEN",
  "auth_header_name": "Authorization",
  "email_header_name": "X-Twist-Email"
}
```

`base_url`, both endpoint paths, and—where necessary—the two header names must
be verified against the account-gated TAPI reference for the account. The CLI
does not guess them.

## IDT

```json
{
  "provider": "idt",
  "client_id_env": "NUCLE_IDT_CLIENT_ID",
  "client_secret_env": "NUCLE_IDT_CLIENT_SECRET",
  "username_env": "NUCLE_IDT_USERNAME",
  "password_env": "NUCLE_IDT_PASSWORD",
  "token_url": "https://www.idtdna.com/Identityserver/connect/token",
  "api_base_url": "https://your-account-api.example",
  "order_path": "/orders",
  "status_path_template": "/orders/{id}"
}
```

`token_url` defaults to the value shown. The order host and paths must be
copied from the account's SciTools Plus documentation.

## Illumina

```json
{
  "provider": "illumina",
  "base_url": "https://api.basespace.illumina.com/v2",
  "access_token_env": "NUCLE_ILLUMINA_ACCESS_TOKEN",
  "launch_path": "/applications/your-app-id/launch"
}
```

`base_url` defaults to the US BaseSpace v2 host shown. This adapter launches
and monitors a BaseSpace secondary-analysis `AppSession`; it does not
commission a physical sequencing run.

## Oxford Nanopore

Developer-token authentication:

```json
{
  "provider": "nanopore",
  "host": "127.0.0.1",
  "manager_port": 9502,
  "ca_certificate_path": "certs/minknow-ca.crt",
  "protocol_identifier": "sequencing/flow-cell-and-kit-specific-id",
  "auth": {
    "type": "developer-api-token",
    "token_env": "NUCLE_NANOPORE_TOKEN"
  }
}
```

Client-certificate authentication:

```json
{
  "provider": "nanopore",
  "host": "instrument-hostname",
  "manager_port": 9501,
  "ca_certificate_path": "certs/minknow-ca.crt",
  "protocol_identifier": "sequencing/flow-cell-and-kit-specific-id",
  "auth": {
    "type": "client-certificate",
    "cert_chain_path": "certs/client.crt",
    "private_key_path": "certs/client.key"
  }
}
```

Certificate paths are resolved relative to the config file. The protocol
identifier must come from the target MinKNOW installation for the loaded flow
cell and kit; there is no safe universal default.
