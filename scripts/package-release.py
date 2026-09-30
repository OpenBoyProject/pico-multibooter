"""Package three PC tools and the common Pico/GBA images for one platform."""

import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import tarfile
import tempfile
import zipfile


TARGETS = (
    "x86_64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
)


def package(args):
    root = Path(__file__).resolve().parent.parent
    suffix = ".exe" if "windows" in args.target else ""
    executables = [name + suffix for name in ("mb-cable", "mb-uploader", "mb-dumper")]
    sources = [args.binaries / name for name in executables]
    sources += [args.images / name for name in (
        "pico-multibooter.uf2", "hello-world_mb.gba", "rom-dumper_mb.gba",
    )]
    for source in sources:
        if not source.is_file() or source.stat().st_size == 0:
            raise ValueError(f"missing or empty release file: {source}")

    name = f"pico-multibooter-{args.tag}-{args.target}"
    args.output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory() as temporary:
        directory = Path(temporary) / name
        directory.mkdir()
        for source in sources:
            destination = directory / source.name
            shutil.copyfile(source, destination)
            # Artifact downloads and Windows builds do not preserve Unix modes.
            destination.chmod(0o755 if source.name in executables else 0o644)
        shutil.copyfile(root / "docs/release-usage.md", directory / "README.md")
        metadata = {"tag": args.tag, "commit": args.commit, "target": args.target}
        (directory / "BUILDINFO.json").write_text(json.dumps(metadata, indent=2) + "\n")
        checksums = []
        for path in sorted(directory.iterdir()):
            digest = hashlib.sha256(path.read_bytes()).hexdigest()
            checksums.append(f"{digest}  {path.name}\n")
        (directory / "SHA256SUMS").write_text("".join(checksums))
        if suffix:
            archive = args.output / f"{name}.zip"
            with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as output:
                for path in sorted(directory.iterdir()):
                    output.write(path, arcname=f"{name}/{path.name}")
        else:
            archive = args.output / f"{name}.tar.gz"
            with tarfile.open(archive, "w:gz") as output:
                output.add(directory, arcname=name)
    print(archive)
    return archive


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--target", required=True, choices=TARGETS)
    parser.add_argument("--binaries", required=True, type=Path)
    parser.add_argument("--images", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    if not re.fullmatch(r"v[0-9][A-Za-z0-9.+-]*", args.tag):
        parser.error("tag must start with v and a digit, using only letters, digits, '.', '+', '-'")
    if not re.fullmatch(r"[0-9a-fA-F]{40}", args.commit):
        parser.error("commit must be a full Git commit hash")
    try:
        package(args)
    except (OSError, ValueError) as error:
        parser.exit(1, f"error: {error}\n")


if __name__ == "__main__":
    main()
