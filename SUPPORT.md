# Getting help

- **How do I…? / Is this a bug?** Ask in [Discussions → Q&A](https://github.com/pdfjiff/pdfjiff/discussions/categories/q-a). Someone may already have asked your question.
- **Something is broken:** open a [bug report](https://github.com/pdfjiff/pdfjiff/issues/new?template=bug_report.yml).
- **Idea or feature request:** open a [feature request](https://github.com/pdfjiff/pdfjiff/issues/new?template=feature_request.yml).
- **Security issue:** follow [SECURITY.md](SECURITY.md). Do not open a public issue.

Before asking, it helps to run:

```bash
pdfjiff --version
pdfjiff capabilities --json      # what this build supports
pdfjiff <command> ... --json     # the exact error code and hint
```

Most failures come with a `hint` that says what to do next. [docs/usage.md](docs/usage.md#error-codes) lists every error code, and [docs/install.md](docs/install.md#troubleshooting) covers installation problems.

PDFJiff is maintained on a best-effort basis. There is no guaranteed response time, but every issue gets read.
