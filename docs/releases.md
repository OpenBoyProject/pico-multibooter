# Release maintenance

The [release workflow](../.github/workflows/release.yml) runs when a tag
starting with `v` is pushed. Use a version such as `v1.0` or `v1.0.0`;
tags such as `v1.0.0-rc.1` are published as prereleases.

Before tagging, update the workspace version in `Cargo.toml` and regenerate
`Cargo.lock` so the CLI `--version` output matches the release. Cargo versions
need three components, for example `1.0.0` for the `v1.0` tag. Commit the
changes, then create and push the tag:

```sh
git tag -a v1.0 -m "Release v1.0"
git push origin v1.0
```

The workflow:

1. Tests and builds the Pico firmware and converts its ELF to RP2040 UF2 using
   picotool 2.2.0 and Pico SDK 2.2.0.
2. Builds and checks both GBA ROMs in the devkitPro `devkitarm:20260610` image.
3. Tests, lints, and builds the PC tools for Linux x64, Windows x64, macOS Intel,
   and macOS Apple Silicon on native runners.
4. Packages the three PC tools and three images for each platform, with
   `README.md`, `BUILDINFO.json`, and per-file `SHA256SUMS`.
5. Attaches the archives, standalone UF2/GBA images, and asset checksums to a
   GitHub release. It publishes the draft only after all uploads succeed.

Unix archives use `.tar.gz` to preserve executable permissions; Windows uses
`.zip` and statically links the C runtime. All archives include the same firmware and GBA images. Linux builds
use Ubuntu 22.04 for a glibc 2.35 baseline. The workflow pins the embedded build
tools and uses current stable Rust with `--locked` dependencies.

Only the publishing job has `contents: write`; it uses the built-in
`GITHUB_TOKEN`. No personal access token is needed. GitHub Actions must be
enabled and repository/organization policy must allow that job permission.

A failed build does not publish a release. A failed upload leaves a draft;
rerunning the workflow can replace its incomplete assets. The workflow refuses
to overwrite an already published release. Use a new tag for corrected builds.

## Local packaging check

Build the PC tools and both GBA ROMs as described in the
[environment setup](../README.md#environment-setup), then prepare the images:

```sh
cargo build --locked --release
cargo build-firmware --locked --release
cargo build-gba
mkdir -p target/release-images
picotool uf2 convert target/thumbv6m-none-eabi/release/firmware -t elf \
  target/release-images/pico-multibooter.uf2 --family rp2040
cp gba/hello-world/build/hello-world_mb.gba target/release-images/
cp gba/rom-dumper/build/rom-dumper_mb.gba target/release-images/
python3 scripts/package-release.py --tag v1.0 --commit "$(git rev-parse HEAD)" \
  --target x86_64-unknown-linux-gnu --binaries target/release \
  --images target/release-images --output target/dist
```

Use the target matching the binaries you actually built. The package script
does not cross-compile them. This command creates an archive only; it does not
tag the repository or publish a release.
