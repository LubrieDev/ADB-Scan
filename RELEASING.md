# Releasing a new version

Pushing a tag `vX.Y.Z` runs [`.github/workflows/release.yml`](.github/workflows/release.yml):
it checks that the tag matches the version in `Cargo.toml`, builds the `.deb`
on Ubuntu 24.04 with `cargo-deb` and publishes a GitHub Release with the
`.deb` attached. Installed copies of ADB Scan see the new release the next
time they start (a banner with a link to the release page).

1. Set the new version in the root `Cargo.toml`, `[workspace.package]`:

   ```toml
   version = "1.1.0"
   ```

2. Build once so `Cargo.lock` picks up the new version. The workflow builds
   with `--locked`, so an outdated `Cargo.lock` makes it fail.

   ```sh
   cargo build
   ```

3. Optional: add a `<release>` entry to
   `packaging/com.adbscan.AdbScan.metainfo.xml`.

4. Commit, tag and push the tag:

   ```sh
   git commit -am "Release 1.1.0"
   git tag v1.1.0
   git push
   git push origin v1.1.0
   ```

5. Follow the run in the repository's **Actions** tab. When it finishes, the
   release is under **Releases**, with notes generated from the commits
   (they can be edited there).

Tags with a suffix, such as `v1.2.0-beta.1`, are published as pre-releases:
the app's update check ignores them.

If the tag and `Cargo.toml` do not match, the workflow stops with an error
and nothing is published. Delete the tag, fix the version and tag again:

```sh
git tag -d v1.1.0
git push origin :refs/tags/v1.1.0
```

## Building the .deb locally

```sh
cargo install cargo-deb   # once
cargo deb -p adb-scan     # → target/debian/adb-scan_<version>-1_amd64.deb
```

The package metadata (maintainer, description, dependencies, files) is in
`[package.metadata.deb]` of `crates/app/Cargo.toml`.
