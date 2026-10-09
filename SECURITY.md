# Security Policy

PDFJiff reads files from untrusted sources, so we take parser and file-handling bugs seriously.

## Supported versions

| Version | Supported |
| --- | --- |
| Latest release | Yes |
| Older releases | No. Please upgrade first. |

## Reporting a vulnerability

**Do not report a vulnerability in a public issue, discussion or pull request.**

Report it privately through GitHub's [private vulnerability reporting](https://github.com/pdfjiff/pdfjiff/security/advisories/new) (open the repository's **Security** tab and choose **Report a vulnerability**). If you cannot use GitHub, email **dev@pdfjiff.com** with the subject `SECURITY: pdfjiff`.

Please include:

- the affected version (`pdfjiff --version`) and platform
- steps to reproduce, ideally with a minimal, synthetic PDF (never a confidential document)
- the impact you observed or expect

## What to expect

- We aim to acknowledge your report within **5 working days**.
- We aim to send a first assessment within **14 days**.
- We fix confirmed issues in a new release and publish a GitHub Security Advisory, crediting you unless you ask not to be named.
- Please give us a reasonable window, normally up to 90 days, before you disclose the issue publicly. We will keep you updated throughout.

## Scope

Examples of issues we treat as vulnerabilities:

- A crafted PDF that causes memory corruption, unbounded resource use beyond the documented limits, or a hang
- Anything that makes PDFJiff modify an input file, overwrite an output without `--overwrite`, or write outside the requested output path
- Tampering with release artifacts, installers, the Docker image or the GitHub Action

Out of scope:

- Detecting or validating digital signatures. `inspect` detects that signatures exist but does not check whether they are valid, and this is documented.
- Memory used by a single large but valid input within the documented byte limits. The limits cap the bytes read, not peak memory.

## Verifying downloads

Every release publishes `SHA256SUMS` and [build provenance attestations](https://docs.github.com/actions/security-for-github-actions/using-artifact-attestations). To confirm that an archive was built by this repository's release workflow:

```bash
gh attestation verify pdfjiff-x86_64-unknown-linux-musl.tar.gz --repo pdfjiff/pdfjiff
```

The installers verify SHA-256 checksums automatically.
