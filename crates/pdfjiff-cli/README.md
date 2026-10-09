# pdfjiff-cli

The `pdfjiff` command-line tool: fast, private PDF compression, merging and inspection that runs entirely on your machine.

```bash
cargo binstall pdfjiff-cli        # prebuilt binary
cargo install pdfjiff-cli --locked  # or build from source
```

```bash
pdfjiff inspect report.pdf
pdfjiff compress report.pdf --target 2MB -o upload.pdf
pdfjiff merge cover.pdf report.pdf -o combined.pdf
pdfjiff capabilities --json
```

Other install options include Homebrew, Scoop, a one-line installer, Docker and a GitHub Action. See the [project README](https://github.com/pdfjiff/pdfjiff#install).

- [Usage reference](https://github.com/pdfjiff/pdfjiff/blob/main/docs/usage.md): every option, the JSON schema, exit codes and error codes
- [Changelog](https://github.com/pdfjiff/pdfjiff/blob/main/CHANGELOG.md)

Licensed under either of [Apache-2.0](https://github.com/pdfjiff/pdfjiff/blob/main/LICENSE-APACHE) or [MIT](https://github.com/pdfjiff/pdfjiff/blob/main/LICENSE-MIT), at your option.
