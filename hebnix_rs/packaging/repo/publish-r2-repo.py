#!/usr/bin/env python3
"""Publish a custom Arch package repo (built from local .pkg.tar.zst files)
to a Cloudflare R2 bucket, S3-compatible.

What it does, in order:
  1. Copies the given package files into a local staging directory.
  2. Runs `repo-add` there to (re)build <repo-name>.db.tar.gz /
     .files.tar.gz -- incrementally, against whatever db already exists
     in the staging dir (downloaded from R2 first, if present), so
     packages that were already in the repo and aren't in this run's
     input directory are kept, not dropped.
  3. Uploads everything under <arch>/ to the bucket, skipping any object
     whose remote content already matches (by MD5) what's about to be
     uploaded -- safe to re-run, only changed/new files actually
     transfer.

pacman only ever requests "<repo-name>.db" / "<repo-name>.files" (no
.tar.gz suffix) -- those are normally local symlinks `repo-add` creates
next to the real archives. S3/R2 has no symlinks, so this script uploads
the same bytes under both the real name and the bare name pacman fetches.

Requires: repo-add (pacman-contrib), boto3, and R2 credentials via env vars:
  R2_ACCOUNT_ID          Cloudflare account ID (the <ACCOUNT_ID> in the R2 endpoint)
  R2_ACCESS_KEY_ID       R2 API token access key
  R2_SECRET_ACCESS_KEY   R2 API token secret key
  R2_BUCKET              bucket name (default: hebnix-linux)

Usage:
  R2_ACCOUNT_ID=... R2_ACCESS_KEY_ID=... R2_SECRET_ACCESS_KEY=... \\
    python3 publish-r2-repo.py --packages-dir ./out

  # Print the pacman.conf snippet for users, then exit (no upload):
  python3 publish-r2-repo.py --print-pacman-conf
"""

from __future__ import annotations

import argparse
import hashlib
import os
import shutil
import subprocess
import sys
from pathlib import Path

REPO_NAME = "hebnix-linux"
ARCH = "x86_64"
DOMAIN = "repo.xplodingeggo.space"


def eprint(*args: object) -> None:
    print(*args, file=sys.stderr)


def pacman_conf_snippet() -> str:
    return (
        f"[{REPO_NAME}]\n"
        f"Server = https://{DOMAIN}/{ARCH}/\n"
    )


def md5sum(path: Path) -> str:
    h = hashlib.md5()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def get_r2_client():
    import boto3
    from botocore.config import Config

    account_id = os.environ.get("R2_ACCOUNT_ID")
    access_key = os.environ.get("R2_ACCESS_KEY_ID")
    secret_key = os.environ.get("R2_SECRET_ACCESS_KEY")
    missing = [
        name
        for name, val in [
            ("R2_ACCOUNT_ID", account_id),
            ("R2_ACCESS_KEY_ID", access_key),
            ("R2_SECRET_ACCESS_KEY", secret_key),
        ]
        if not val
    ]
    if missing:
        eprint(f"error: missing required env var(s): {', '.join(missing)}")
        sys.exit(1)

    endpoint = f"https://{account_id}.r2.cloudflarestorage.com"
    return boto3.client(
        "s3",
        endpoint_url=endpoint,
        aws_access_key_id=access_key,
        aws_secret_access_key=secret_key,
        config=Config(
            signature_version="s3v4",
            retries={"max_attempts": 5, "mode": "standard"},
            # R2 needs path-style addressing (endpoint/bucket/key); boto3
            # defaults to virtual-hosted style (bucket.endpoint/key), which
            # doesn't resolve against R2's account-scoped endpoint.
            s3={"addressing_style": "path"},
        ),
        region_name="auto",
    )


def download_existing_db(client, bucket: str, staging: Path) -> None:
    """Pull the current db/files archives (if any) into staging so
    repo-add updates incrementally instead of starting from empty."""
    for name in (f"{REPO_NAME}.db.tar.gz", f"{REPO_NAME}.files.tar.gz"):
        key = f"{ARCH}/{name}"
        dest = staging / name
        try:
            client.download_file(bucket, key, str(dest))
            print(f"fetched existing {key} -> {dest}")
        except Exception as e:  # noqa: BLE001 - boto3 raises varied client errors
            print(f"no existing {key} in bucket yet ({e.__class__.__name__}), starting fresh")


def run_repo_add(staging: Path, package_files: list[Path]) -> None:
    if not package_files:
        eprint("error: no .pkg.tar.zst files found to add")
        sys.exit(1)

    db_path = staging / f"{REPO_NAME}.db.tar.gz"
    cmd = ["repo-add", "--remove", str(db_path)] + [str(p) for p in package_files]
    print(f"$ {' '.join(cmd)}")
    subprocess.run(cmd, cwd=staging, check=True)


def resolve_db_outputs(staging: Path) -> dict[str, Path]:
    """repo-add produces <repo>.db.tar.gz/.files.tar.gz plus <repo>.db/
    .files symlinks pointing at them. Return {upload_name: real_file}
    for both the real names and the bare names pacman actually fetches."""
    outputs: dict[str, Path] = {}
    for kind in ("db", "files"):
        real = staging / f"{REPO_NAME}.{kind}.tar.gz"
        bare = staging / f"{REPO_NAME}.{kind}"
        if not real.exists():
            eprint(f"error: repo-add didn't produce {real}")
            sys.exit(1)
        outputs[real.name] = real
        # bare name: same bytes, resolve the symlink repo-add makes (or
        # just reuse `real` directly if for some reason it isn't a symlink)
        outputs[bare.name] = real if not bare.is_symlink() else real
    return outputs


def remote_md5(client, bucket: str, key: str) -> str | None:
    try:
        head = client.head_object(Bucket=bucket, Key=key)
    except Exception:  # noqa: BLE001
        return None
    etag = head.get("ETag", "").strip('"')
    # ETag is only a plain MD5 for non-multipart uploads (true for
    # everything this script uploads -- these files are small enough
    # that boto3's default TransferConfig won't multipart them).
    if "-" in etag:
        return None
    return etag


def upload_file(client, bucket: str, key: str, local_path: Path, *, force: bool) -> bool:
    """Returns True if actually uploaded, False if skipped (already
    up to date)."""
    if not force:
        local_hash = md5sum(local_path)
        existing = remote_md5(client, bucket, key)
        if existing == local_hash:
            print(f"skip  {key} (unchanged)")
            return False

    content_type = "application/octet-stream"
    if key.endswith(".files") or key.endswith(".files.tar.gz"):
        content_type = "application/gzip" if key.endswith(".tar.gz") else "application/octet-stream"

    print(f"push  {key}")
    client.upload_file(str(local_path), bucket, key, ExtraArgs={"ContentType": content_type})
    return True


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument(
        "--packages-dir",
        type=Path,
        help="directory containing built *.pkg.tar.zst files to publish",
    )
    parser.add_argument(
        "--include-debug",
        action="store_true",
        help="also publish *-debug-*.pkg.tar.zst packages (skipped by default: large, rarely needed)",
    )
    parser.add_argument(
        "--bucket",
        default=os.environ.get("R2_BUCKET", REPO_NAME),
        help="R2 bucket name (default: $R2_BUCKET or 'hebnix-linux')",
    )
    parser.add_argument(
        "--force",
        action="store_true",
        help="re-upload every file regardless of whether the remote copy already matches",
    )
    parser.add_argument(
        "--print-pacman-conf",
        action="store_true",
        help="print the pacman.conf snippet for users and exit (no upload)",
    )
    args = parser.parse_args()

    if args.print_pacman_conf:
        print(pacman_conf_snippet())
        return

    if not args.packages_dir:
        parser.error("--packages-dir is required unless --print-pacman-conf is given")

    if not shutil.which("repo-add"):
        eprint("error: repo-add not found (install pacman-contrib)")
        sys.exit(1)

    packages_dir: Path = args.packages_dir
    if not packages_dir.is_dir():
        eprint(f"error: {packages_dir} is not a directory")
        sys.exit(1)

    package_files = sorted(packages_dir.glob("*.pkg.tar.zst"))
    if not args.include_debug:
        package_files = [p for p in package_files if "-debug-" not in p.name]
    if not package_files:
        eprint(f"error: no *.pkg.tar.zst files found in {packages_dir}")
        sys.exit(1)
    print(f"found {len(package_files)} package(s): {', '.join(p.name for p in package_files)}")

    client = get_r2_client()
    bucket = args.bucket

    staging = Path.cwd() / f".{REPO_NAME}-repo-staging"
    if staging.exists():
        shutil.rmtree(staging)
    staging.mkdir(parents=True)

    try:
        download_existing_db(client, bucket, staging)

        staged_packages = []
        for pkg in package_files:
            dest = staging / pkg.name
            shutil.copy2(pkg, dest)
            staged_packages.append(dest)

        run_repo_add(staging, staged_packages)

        db_outputs = resolve_db_outputs(staging)

        uploaded = 0
        skipped = 0
        for pkg in staged_packages:
            key = f"{ARCH}/{pkg.name}"
            if upload_file(client, bucket, key, pkg, force=args.force):
                uploaded += 1
            else:
                skipped += 1

        # db/files archives + their bare-name duplicates always get
        # re-uploaded when their content actually changed (repo-add
        # rewrites them on every run even if nothing meaningfully
        # changed inside, so the md5 check above still keeps this cheap
        # when this script is re-run with the same package set).
        for name, real_path in db_outputs.items():
            key = f"{ARCH}/{name}"
            if upload_file(client, bucket, key, real_path, force=args.force):
                uploaded += 1
            else:
                skipped += 1

        print(f"\ndone: {uploaded} uploaded, {skipped} unchanged/skipped")
        print("\nAdd this to /etc/pacman.conf to use the repo:\n")
        print(pacman_conf_snippet())
    finally:
        shutil.rmtree(staging, ignore_errors=True)


if __name__ == "__main__":
    main()
