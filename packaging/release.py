"""Build and inspect the small, self-contained release archives."""

import argparse
import hashlib
import io
from pathlib import Path
import re
import subprocess
import sys
import tarfile
import zipfile


TARGETS = (
    "x86_64-unknown-linux-musl",
    "aarch64-unknown-linux-musl",
    "x86_64-pc-windows-msvc",
    "aarch64-pc-windows-msvc",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
)


def entries(binary: Path, licenses: Path) -> dict[str, bytes]:
    return {
        binary.name: binary.read_bytes(),
        "README.txt": Path("README.txt").read_bytes(),
        "LICENSE": Path("LICENSE").read_bytes(),
        "THIRD_PARTY_LICENSES.html": licenses.read_bytes(),
    }


def archive(binary: Path, licenses: Path, output: Path) -> None:
    files = entries(binary, licenses)
    if len(files) != 4 or any(not data for data in files.values()):
        raise ValueError("release archive needs four nonempty files")
    executable = files[binary.name]
    if (
        b"YETTEL_CWMP_DEV_PROVIDER" in executable
        or b"dev build" in executable.lower()
        or b"RAZVOJNA VERZIJA" in executable
    ):
        raise ValueError("development override is present in the release executable")
    output.parent.mkdir(parents=True, exist_ok=True)
    if output.name.endswith(".tar.gz"):
        with tarfile.open(output, "w:gz") as handle:
            for name, data in files.items():
                info = tarfile.TarInfo(name)
                info.size = len(data)
                info.mode = 0o755 if name == binary.name else 0o644
                handle.addfile(info, io.BytesIO(data))
    elif output.suffix == ".zip":
        with zipfile.ZipFile(output, "w", zipfile.ZIP_DEFLATED) as handle:
            for name, data in files.items():
                handle.writestr(name, data)
    else:
        raise ValueError(f"unsupported archive type: {output}")
    check_archive(output)


def check_archive(path: Path) -> None:
    binary = "yettel-cwmp.exe" if path.suffix == ".zip" else "yettel-cwmp"
    expected = {binary, "README.txt", "LICENSE", "THIRD_PARTY_LICENSES.html"}
    if path.suffix == ".zip":
        with zipfile.ZipFile(path) as handle:
            names = handle.namelist()
            sizes = [handle.getinfo(name).file_size for name in names]
    else:
        with tarfile.open(path, "r:gz") as handle:
            members = handle.getmembers()
            names = [member.name for member in members]
            sizes = [member.size for member in members]
    if len(names) != 4 or set(names) != expected or any(size == 0 for size in sizes):
        raise ValueError(f"unexpected contents in {path}: {names}")


def checksums(directory: Path) -> None:
    archives = sorted((*directory.glob("*.tar.gz"), *directory.glob("*.zip")))
    if len(archives) != 5:
        raise ValueError(f"expected five release archives, found {len(archives)}")
    for path in archives:
        check_archive(path)
    lines = [f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.name}" for path in archives]
    (directory / "SHA256SUMS").write_text("\n".join(lines) + "\n", encoding="ascii")


def notes(version: str, output: Path) -> None:
    text = Path("CHANGELOG.md").read_text(encoding="utf-8")
    heading = re.compile(rf"^## \[{re.escape(version)}\](?:\s+-\s+.*)?\s*$", re.M)
    match = heading.search(text)
    if not match:
        raise ValueError(f"CHANGELOG.md has no {version} section")
    next_heading = re.search(r"^## ", text[match.end():], re.M)
    end = match.end() + next_heading.start() if next_heading else len(text)
    body = text[match.end():end].strip()
    if not body:
        raise ValueError(f"CHANGELOG.md {version} section is empty")
    output.write_text(body + "\n", encoding="utf-8")


def verify_licenses(path: Path) -> None:
    html = path.read_text(encoding="utf-8")
    missing = set()
    for target in TARGETS:
        result = subprocess.run(
            ["cargo", "tree", "--locked", "-e", "normal", "--target", target,
             "--prefix", "none", "--format", "{p}"],
            check=True, capture_output=True, text=True,
        )
        for line in result.stdout.splitlines():
            match = re.match(r"([^ ]+) v([^ ]+)", line)
            if match and match.group(1) != "yettel-cwmp":
                label = f"{match.group(1)} {match.group(2)}"
                if f"<li>{label}</li>" not in html:
                    missing.add(label)
    if missing:
        raise ValueError("license notice is missing: " + ", ".join(sorted(missing)))


def main() -> None:
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="command", required=True)
    create = sub.add_parser("archive")
    create.add_argument("binary", type=Path)
    create.add_argument("licenses", type=Path)
    create.add_argument("output", type=Path)
    inspect = sub.add_parser("check")
    inspect.add_argument("archive", type=Path)
    sums = sub.add_parser("checksums")
    sums.add_argument("directory", type=Path)
    changelog = sub.add_parser("notes")
    changelog.add_argument("version")
    changelog.add_argument("output", type=Path)
    licenses = sub.add_parser("verify-licenses")
    licenses.add_argument("html", type=Path)
    args = parser.parse_args()
    if args.command == "archive":
        archive(args.binary, args.licenses, args.output)
    elif args.command == "check":
        check_archive(args.archive)
    elif args.command == "checksums":
        checksums(args.directory)
    elif args.command == "notes":
        notes(args.version, args.output)
    else:
        verify_licenses(args.html)


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        sys.exit(str(error))
