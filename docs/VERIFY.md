# Verify a release binary against this repository

Interpres is **public open source**. Release builds are **not obfuscated**. Curious users and developers can compare what you download with what is on GitHub.

## What “transparent build” means here

1. **Source of truth** is this repository (readable Rust, helpers, packaging scripts).
2. Each GitHub **Release** is tagged to an exact **git commit**.
3. Binaries are normal Rust `release` builds (optimized, but **not packed/encrypted/obfuscated**).
4. You can rebuild the same way and compare checksums, or inspect the binary with normal tools (`strings`, `otool`, `objdump`, reverse-engineering tools of your choice).

We do **not** try to make reverse engineering hard. Privacy for *your transcripts* is local files and opt-in save — not binary secrecy.

## Check the tag matches the code

```bash
git clone https://github.com/IronAdamant/Interpres.git
cd Interpres
git fetch --tags
git checkout v0.2.0   # or the tag you downloaded
git rev-parse HEAD    # should match the commit shown on the Release page
```

## Rebuild yourself (recommended audit)

```bash
cargo build --release
# optional portable folder:
./packaging/make-double-click.sh
shasum -a 256 dist/Interpres/interpres
```

Compare that SHA-256 to `SHA256SUMS.txt` attached to the GitHub Release.

## Inspect a downloaded Mac binary

```bash
# after unzipping the release asset
shasum -a 256 interpres
file interpres
strings interpres | head
# symbols / linkage (macOS)
otool -L interpres
nm -gU interpres | head
```

Rust release builds strip some debug detail for size, but they are still ordinary machine code linked to system libraries — not a sealed black box. The **authoritative** “what’s inside” answer is always the **source tree** at the release tag.

## Windows

When a Windows `.exe` is attached to a Release, the same idea applies: rebuild with `cargo build --release` on Windows, compare hashes, inspect with your preferred tools. If only a Mac build is published for a tag, use source + your own Windows build until a CI Windows artifact is available.

## License

MIT OR Apache-2.0 — fork, rebuild, and redistribute under those terms.
