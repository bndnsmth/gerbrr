# Native releases

Push a version tag to build four native downloads on GitHub-hosted runners:

| Download | Build runner |
| --- | --- |
| `gerbrr-vVERSION-linux-x86_64.tar.gz` | Ubuntu 22.04 |
| `gerbrr-vVERSION-windows-x86_64.zip` | Windows Server 2022 |
| `gerbrr-vVERSION-macos-arm64.zip` | macOS 15, Apple Silicon |
| `gerbrr-vVERSION-macos-x86_64.zip` | macOS 15, Intel |

macOS downloads contain **Gerbrr.app**. Every package includes launch instructions,
the project license, and dependency/font/icon license notices. The release also
contains **SHA256SUMS**. No board archives, account settings, or deployment
credentials are packaged.

## First: activate and test the workflow

Commit and push the workflow and its helpers to `main`. Then open the repository's
**Actions → Native release artifacts → Run workflow**, choosing `main`.

This manual run builds and tests all four platforms and uploads packages under
the run's **Artifacts** section, but **does not create a GitHub Release**. Download
the packages and try the GUI on the relevant machines before claiming support.
Automated checks cover parser/geometry/UI regressions and the executable's
`--version` and built-in demo import; they do not replace testing a real desktop.

## Cut a release

1. Set `[package].version` in `Cargo.toml` to the desired version. For the first
   release it is already **0.1.0**. For later bumps, run `cargo check` to update the
   root package version in `Cargo.lock`, and commit both files.
2. Commit any release changes, push `main`, and tag that exact commit:

   ```sh
   git push origin main
   git tag -a v0.1.0 -m "Gerbrr 0.1.0"
   git push origin v0.1.0
   ```

3. Open **Actions → Native release artifacts** to follow the four builds.
4. After every job passes, open **Releases**. A **draft Gerbrr 0.1.0** release
   appears with all downloads and checksums. Review the notes, mention unsigned
   builds and platform requirements, then click **Publish release**.

The tag must exactly match `Cargo.toml`: `v0.1.0` cannot publish a `0.2.0` binary.
Prerelease versions such as `0.2.0-rc.1` use `v0.2.0-rc.1` and are marked as
prereleases. Do not move published tags; release a new version instead.

**While the repository is private, its releases and artifacts are private too.**
Standard hosted runners use the private repository's Actions allowance; public
repositories get standard hosted runners free.

## How it works

`.github/workflows/release.yml` is separate from the website workflow:

- Uses the existing mise-pinned Rust and Node versions, locked Cargo dependencies,
  and pinned GitHub Actions. Native builds run on their own OS/architecture.
- Runs native regression tests before `cargo build --release --locked`.
- Generates third-party notices with pinned `cargo-about` and packages only
  explicitly selected release files.
- Verifies the built binary's version and demo import, then hashes its archive.
- Gives repository-write permission only to the final release job. It verifies
  all four checksums and that the remote tag still points to the built commit.
- Creates drafts only. Re-running can refresh an existing draft, but cannot
  overwrite a published release.

GitHub provides the workflow's scoped `GITHUB_TOKEN` automatically. No personal
access token, Cloudflare token, or account ID is needed. Creating a native release
does not redeploy the website or change repository visibility.

## Requirements and signing

- **Linux:** glibc 2.35+ and a compatible desktop graphics stack/driver.
- **Windows:** x86-64 Windows with a compatible graphics driver; the EXE is unsigned.
- **macOS:** macOS 13+; separate ARM64 and Intel bundles. Bundles are ad-hoc signed
  for integrity, **not** Developer ID signed or notarized.

Unsigned/not-notarized downloads may trigger Windows or macOS security warnings.
Signing certificates, installer packaging, and notarization can be added later.
Never ask users to disable system-wide security settings.

## Local validation

```sh
mise run release-check
node scripts/release.mjs info
```

To build a native Linux package yourself (on x86-64 Linux):

```sh
rustup target add x86_64-unknown-linux-gnu
cargo build --release --locked --bin gerbrr --target x86_64-unknown-linux-gnu
cargo install cargo-about --version 0.9.2 --features cli --locked --root target/tools
mkdir -p target/release-notices
./target/tools/bin/cargo-about generate --locked --fail --target x86_64-unknown-linux-gnu --output-file target/release-notices/x86_64-unknown-linux-gnu.html release/licenses.hbs
node scripts/release.mjs package x86_64-unknown-linux-gnu
```

The archive and its `.sha256` file appear under `target/release-artifacts/`.
Packaging refuses to overwrite an existing archive. Build Windows/macOS packages
on the matching native OS rather than relabeling a Linux executable.
