#!/usr/bin/env python3
"""
Publish all workspace crates to crates.io in dependency order.
Skips crates whose current version is already published.

Usage:
    python3 scripts/publish.py            # publish for real
    python3 scripts/publish.py --dry-run  # just show what would happen
"""
import re
import subprocess
import sys
import time
from urllib.request import urlopen
from urllib.error import HTTPError

# Topological order — every crate's dependencies come before it.
CRATES = [
    "crabtalk-core",
    "crabtalk-command-codegen",
    "crabtalk-transport",
    "crabtalk-model",
    "crabtalk-plugins",
    "crabtalk-command",
    "crabtalk-gateway",
    "crabtalk-runtime",
    "crabtalk-daemon",
    "crabtalk-outlook",
    "crabtalk-search",
    "crabtalk-telegram",
    "crabtalk-wechat",
    "crabtalk",
]


def workspace_version():
    with open("Cargo.toml") as f:
        in_workspace_pkg = False
        for line in f:
            if line.strip() == "[workspace.package]":
                in_workspace_pkg = True
                continue
            if in_workspace_pkg:
                if line.startswith("["):
                    break
                m = re.match(r'^version\s*=\s*"(.+)"', line)
                if m:
                    return m.group(1)
    sys.exit("error: could not find version in [workspace.package]")


def is_published(crate, version):
    """Check the crates.io API for an exact version."""
    url = f"https://crates.io/api/v1/crates/{crate}/{version}"
    try:
        urlopen(url)
        return True
    except HTTPError:
        return False


def publish(crate):
    subprocess.run(["cargo", "publish", "-p", crate], check=True)


def main():
    dry_run = "--dry-run" in sys.argv
    version = workspace_version()
    published = 0
    skipped = 0

    for crate in CRATES:
        tag = f"{crate}@{version}"
        if is_published(crate, version):
            print(f"skip  {tag} (already published)")
            skipped += 1
            continue

        if dry_run:
            print(f"would publish {tag}")
            published += 1
            continue

        print(f"publish {tag} ...")
        publish(crate)
        published += 1

        # crates.io needs a moment to index before dependents can resolve.
        print("waiting for index ...")
        time.sleep(15)

    print(f"\ndone: {published} published, {skipped} skipped")


if __name__ == "__main__":
    main()
