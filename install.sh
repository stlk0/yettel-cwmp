#!/bin/sh
# Installs the latest yettel-cwmp release on macOS or Linux and starts it:
#
#   curl -fsSL https://raw.githubusercontent.com/stlk0/yettel-cwmp/main/install.sh | sh
#
# curl, unlike a browser, does not quarantine the download, so macOS runs the
# unsigned binary without a Gatekeeper prompt. The archive is checked against
# the release's SHA256SUMS before anything is installed.
set -eu

releases=https://github.com/stlk0/yettel-cwmp/releases/latest/download
bin_dir=$HOME/.local/bin
doc_dir=$HOME/.local/share/doc/yettel-cwmp

fail() {
  echo "yettel-cwmp install: $*" >&2
  exit 1
}

# Everything runs from main so that a download cut short by the network
# executes nothing.
main() {
  case "$(uname -s)-$(uname -m)" in
    Darwin-*) platform=macos-universal ;;
    Linux-x86_64 | Linux-amd64) platform=linux-x64 ;;
    Linux-aarch64 | Linux-arm64) platform=linux-arm64 ;;
    *) fail "no release for $(uname -s) $(uname -m); see https://github.com/stlk0/yettel-cwmp#download" ;;
  esac

  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT
  cd "$tmp"

  curl -fsSLO "$releases/SHA256SUMS"
  grep -- "-$platform\.tar\.gz\$" SHA256SUMS > sum || fail "the latest release has no $platform archive"
  archive=$(sed 's/^[0-9a-f]*  //' sum)
  echo "Downloading $archive"
  curl -fsSLO "$releases/$archive"
  if command -v sha256sum > /dev/null; then
    sha256sum -c sum > /dev/null || fail "$archive does not match SHA256SUMS"
  else
    shasum -a 256 -c sum > /dev/null || fail "$archive does not match SHA256SUMS"
  fi
  tar -xzf "$archive"

  mkdir -p "$bin_dir" "$doc_dir"
  cp README.txt LICENSE THIRD_PARTY_LICENSES.html "$doc_dir/"
  # Replace by rename: overwriting a binary in place can make macOS kill it
  # and makes Linux refuse while it is running.
  cp yettel-cwmp "$bin_dir/.yettel-cwmp.new"
  chmod 755 "$bin_dir/.yettel-cwmp.new"
  mv -f "$bin_dir/.yettel-cwmp.new" "$bin_dir/yettel-cwmp"

  echo "Installed $("$bin_dir/yettel-cwmp" --version) to $bin_dir/yettel-cwmp"
  echo "License notices: $doc_dir"
  case ":$PATH:" in
    *":$bin_dir:"*) echo "Start it with: yettel-cwmp" ;;
    *) echo "Start it with: $bin_dir/yettel-cwmp" ;;
  esac

  # With curl | sh this script arrives on stdin, so give the app the terminal
  # that stdout writes to. Not /dev/tty: macOS kqueue cannot poll it, and the
  # app's input reader fails. Without a terminal (CI, cron), only install.
  if [ -t 1 ]; then
    "$bin_dir/yettel-cwmp" <&1
  fi
}

main
