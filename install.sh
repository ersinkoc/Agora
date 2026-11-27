#!/bin/sh
# Installs the newest Agora release for this user on Linux or macOS, checked before anything changes:
#
#   curl -fsSL https://raw.githubusercontent.com/ersinkoc/Agora/main/install.sh | sh
#   curl -fsSL https://raw.githubusercontent.com/ersinkoc/Agora/main/install.sh | sh -s -- --version 0.3.43 --service
#
# - Downloads the program for this system from the release, with its `.sha256` and `.sig`, and checks both: the checksum, and the
#   signature against the release key below (`ssh-keygen -Y verify`, the same check `agora update` makes).
# - Puts it in ~/.local/bin (or over the `agora` already on PATH, when that one is yours), never as root, adds the folder to PATH in
#   your shell's startup file when it is missing, and pins the release key in <AGORA_HOME>/release-key so `agora update` can check
#   later releases.
# - On a machine already set up, offers to start Agora at login (`agora service install`); on a new one, `agora` asks that itself.
#   Nothing running is stopped or restarted.
#
# Options: --version X.Y.Z, --dir DIR, --no-modify-path, --service, --no-service, --unsigned (only when ssh-keygen is missing), --help.
# The same through the environment: AGORA_VERSION, AGORA_INSTALL_DIR.
#
# The whole script is one function called on its last line, so a download cut short runs nothing.

# The product's name and where it is published: the one place they appear in this script.
BIN=agora
REPO=ersinkoc/Agora
# docs/security/release-key.pub (scripts/check-docs.sh keeps the two the same).
RELEASE_KEY='agora-release ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAuO/WmQDYmsECh5NeOs8iIwdY56M9/CcV94HGYmfiAK'

install_main() {
    set -eu
    base="https://github.com/$REPO/releases"
    version="${AGORA_VERSION:-}"
    dir="${AGORA_INSTALL_DIR:-}"
    modify_path=1
    service=ask
    unsigned=0
    while [ $# -gt 0 ]; do
        case "$1" in
            --version) [ $# -ge 2 ] || die "--version needs a version"; version="$2"; shift 2 ;;
            --version=*) version="${1#--version=}"; shift ;;
            --dir) [ $# -ge 2 ] || die "--dir needs a folder"; dir="$2"; shift 2 ;;
            --dir=*) dir="${1#--dir=}"; shift ;;
            --no-modify-path) modify_path=0; shift ;;
            --service) service=yes; shift ;;
            --no-service) service=no; shift ;;
            --unsigned) unsigned=1; shift ;;
            -h|--help) usage; return 0 ;;
            *) die "unknown option: $1 (see --help)" ;;
        esac
    done
    version="${version#v}"

    if [ "$(id -u)" = 0 ] && [ -z "$dir" ]; then
        die "run this as the user who will use $BIN, not as root ($BIN runs per user; to install for root anyway, pass --dir)"
    fi
    [ -n "${HOME:-}" ] || die "HOME is not set"

    target=$(detect_target)
    have curl || have wget || die "curl or wget is needed"

    if [ -z "$version" ]; then
        version=$(latest_version) || die "could not find the newest release at $base/latest"
    fi
    case "$version" in
        ''|*[!0-9.]*) die "not a release version: $version" ;;
    esac

    name="$BIN-$target"
    url="$base/download/v$version/$name"
    tmp=$(mktemp -d 2>/dev/null || mktemp -d -t "$BIN-install")
    trap 'rm -rf "$tmp"' EXIT
    trap 'rm -rf "$tmp"; exit 130' INT TERM

    say "Downloading $BIN $version for $target"
    fetch "$url" "$tmp/$name" || die "download failed: $url"
    fetch "$url.sha256" "$tmp/$name.sha256" || die "download failed: $url.sha256"

    want=$(awk '{print tolower($1); exit}' "$tmp/$name.sha256")
    got=$(sha256_of "$tmp/$name")
    if [ "${#want}" -ne 64 ] || [ "$want" != "$got" ]; then
        die "the checksum does not match: the download is damaged or not the release (nothing was installed)"
    fi

    if have ssh-keygen && keygen_signs; then
        fetch "$url.sig" "$tmp/$name.sig" || die "download failed: $url.sig"
        printf '%s\n' "$RELEASE_KEY" > "$tmp/allowed_signers"
        if ! ssh-keygen -Y verify -f "$tmp/allowed_signers" -I agora-release -n file -s "$tmp/$name.sig" \
            < "$tmp/$name" > /dev/null 2>&1; then
            die "the release signature does not verify (nothing was installed)"
        fi
        say "Checksum and release signature verified"
    elif [ "$unsigned" = 1 ]; then
        say "warning: no ssh-keygen that checks signatures (OpenSSH 8.1 or newer): only the checksum was checked (--unsigned)"
    else
        die "ssh-keygen from OpenSSH 8.1 or newer is needed to check the release signature (it is missing or too old); install or update OpenSSH, or pass --unsigned to rely on the checksum alone"
    fi

    # Where it goes: --dir; else over the copy already on PATH when it is this user's (a login service runs that path); else ~/.local/bin.
    previous=$(command -v "$BIN" 2>/dev/null || true)
    case "$previous" in /*) ;; *) previous= ;; esac
    if [ -z "$dir" ]; then
        # A copy a package manager owns (Homebrew, Nix) stays its own: installing over it would conflict with its next upgrade.
        case "$previous" in */Cellar/*|*/homebrew/*|*/linuxbrew/*|/nix/*) previous_managed=1 ;; *) previous_managed=0 ;; esac
        if [ -n "$previous" ] && [ "$previous_managed" = 0 ] && [ -w "$(dirname "$previous")" ] && [ -O "$previous" ]; then
            dir=$(dirname "$previous")
        else
            dir="$HOME/.local/bin"
        fi
    fi
    mkdir -p "$dir"
    old_version=""
    if [ -x "$dir/$BIN" ]; then
        old_version=$("$dir/$BIN" --version 2>/dev/null | awk '{print $NF; exit}' || true)
    fi
    # A new file beside the old one, then a rename: a running Agora keeps its program until it restarts.
    cp "$tmp/$name" "$dir/.$BIN.new.$$"
    chmod 755 "$dir/.$BIN.new.$$"
    mv -f "$dir/.$BIN.new.$$" "$dir/$BIN"
    say "Installed $dir/$BIN ($("$dir/$BIN" --version 2>/dev/null || echo "$version"))"

    if [ -n "$previous" ] && [ "$previous" != "$dir/$BIN" ]; then
        say "note: another $BIN is at $previous; whichever folder comes first in PATH wins"
    fi

    path_note=""
    case ":$PATH:" in
        *":$dir:"*) ;;
        *)
            if [ "$modify_path" = 1 ]; then
                if add_to_path "$dir"; then
                    path_note="Open a new terminal (or run: export PATH=\"$dir:\$PATH\") to use $BIN."
                else
                    path_note="$dir is not on PATH: add it, or run $dir/$BIN."
                fi
            else
                path_note="$dir is not on PATH: add it, or run $dir/$BIN."
            fi
            ;;
    esac

    pin_release_key

    home="${AGORA_HOME:-$HOME/.agora}"
    if [ -f "$home/machine.toml" ]; then
        if [ "$service" = ask ]; then
            service=no
            if [ -r /dev/tty ] && [ -w /dev/tty ]; then
                printf 'Start %s when you log in? [Y/n] ' "$BIN" > /dev/tty
                answer=""
                read -r answer < /dev/tty || answer=n
                case "$answer" in ''|y|Y|yes|YES|Yes) service=yes ;; esac
            fi
        fi
        if [ "$service" = yes ]; then
            # Idempotent: a service set up already is left as it is (and never restarted).
            "$dir/$BIN" service install || say "warning: '$BIN service install' did not finish; run it again to see why"
        fi
        if [ -n "$old_version" ] && [ "$old_version" != "$version" ]; then
            say "$BIN keeps running $old_version until it restarts. Your agents keep running either way:"
            if [ "$(uname -s)" = Linux ]; then
                say "  systemctl --user restart $BIN-hub $BIN-node    (a login service; only the roles this machine runs)"
            fi
            say "  $BIN down, then $BIN                              (started by hand)"
        fi
    else
        say "Next: run '$BIN' to set this machine up (it asks whether to start $BIN when you log in)."
    fi
    [ -z "$path_note" ] || say "$path_note"
}

usage() {
    cat <<EOF
Installs the newest $BIN release for this user, after checking its checksum and release signature.

  --version X.Y.Z    install this release instead of the newest (AGORA_VERSION)
  --dir DIR          install into DIR (AGORA_INSTALL_DIR; default: over your $BIN on PATH, else ~/.local/bin)
  --no-modify-path   do not add DIR to PATH in your shell's startup file
  --service          start $BIN at login on a machine already set up, without asking
  --no-service       do not ask about starting $BIN at login
  --unsigned         without ssh-keygen, rely on the checksum alone
EOF
}

say() { printf '%s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }
have() { command -v "$1" > /dev/null 2>&1; }

detect_target() {
    os=$(uname -s)
    arch=$(uname -m)
    case "$os" in
        Linux)
            if have ldd && ldd --version 2>&1 | grep -qi musl; then
                die "this Linux uses musl libc (Alpine?); the release is built for glibc: build from source instead"
            fi
            case "$arch" in
                x86_64|amd64) echo x86_64-unknown-linux-gnu ;;
                aarch64|arm64) echo aarch64-unknown-linux-gnu ;;
                *) die "no release for Linux on $arch" ;;
            esac
            ;;
        Darwin)
            # A shell under Rosetta says x86_64 on Apple silicon: take the native build.
            if [ "$arch" = x86_64 ] && [ "$(sysctl -n sysctl.proc_translated 2>/dev/null || echo 0)" = 1 ]; then
                arch=arm64
            fi
            case "$arch" in
                arm64|aarch64) echo aarch64-apple-darwin ;;
                x86_64) echo x86_64-apple-darwin ;;
                *) die "no release for macOS on $arch" ;;
            esac
            ;;
        MINGW*|MSYS*|CYGWIN*) die "on Windows use install.ps1 in PowerShell" ;;
        *) die "no release for $os" ;;
    esac
}

fetch() {
    if have curl; then
        curl --fail --silent --show-error --location --proto '=https' --tlsv1.2 --retry 3 --max-time 300 -o "$2" "$1"
    else
        wget --quiet --https-only --tries=3 --timeout=300 -O "$2" "$1"
    fi
}

# The newest release's version, from where `releases/latest` redirects (no API call, no rate limit).
latest_version() {
    if have curl; then
        where=$(curl --fail --silent --show-error --location --proto '=https' --tlsv1.2 --max-time 60 \
            -o /dev/null -w '%{url_effective}' "$base/latest")
    else
        where=$(wget --https-only --max-redirect=5 --server-response --spider "$base/latest" 2>&1 \
            | awk '/^ *[Ll]ocation: /{l=$2} END{print l}' | tr -d '\r')
    fi
    tag=${where%/}
    tag=${tag##*/}
    v=${tag#v}
    case "$tag" in
        v*) ;;
        *) return 1 ;;
    esac
    case "$v" in
        ''|*[!0-9.]*) return 1 ;;
    esac
    echo "$v"
}

# Whether this ssh-keygen checks signatures (`-Y`, OpenSSH 8.1 and newer); an older one says "illegal option -- Y".
keygen_signs() {
    ! ssh-keygen -Y verify < /dev/null 2>&1 | grep -q 'option -- Y'
}

sha256_of() {
    if have sha256sum; then
        sha256sum "$1" | awk '{print tolower($1)}'
    elif have shasum; then
        shasum -a 256 "$1" | awk '{print tolower($1)}'
    else
        die "sha256sum or shasum is needed to check the download"
    fi
}

# Adds `$1` to PATH in the startup file of the user's shell (and ~/.profile for login shells), once. Only a folder name with nothing
# a shell would read as code is written there; for any other, the person is told the line to add.
add_to_path() {
    case "$1" in
        *[!A-Za-z0-9\ ._/+@%,~-]*)
            say "note: $1 has characters this installer does not write into a shell startup file: add it to PATH yourself"
            return 1
            ;;
    esac
    marker="# added by the $BIN installer"
    # Under the home folder it is written as $HOME/..., so the line still holds if the home folder moves.
    case "$1" in
        "$HOME"/*) shown="\$HOME${1#"$HOME"}" ;;
        *) shown="$1" ;;
    esac
    line="case \":\$PATH:\" in *\":$shown:\"*) ;; *) export PATH=\"$shown:\$PATH\" ;; esac $marker"
    files=""
    case "$(basename "${SHELL:-sh}")" in
        bash)
            files="$HOME/.bashrc"
            # macOS opens login shells, which read ~/.bash_profile, not ~/.bashrc.
            [ "$(uname -s)" = Darwin ] && files="$files $HOME/.bash_profile"
            ;;
        zsh) files="${ZDOTDIR:-$HOME}/.zshrc" ;;
        fish)
            fish_file="${XDG_CONFIG_HOME:-$HOME/.config}/fish/conf.d/$BIN.fish"
            if [ ! -f "$fish_file" ]; then
                mkdir -p "$(dirname "$fish_file")"
                printf 'fish_add_path "%s" %s\n' "$shown" "$marker" > "$fish_file"
                say "Added $1 to PATH in $fish_file"
            fi
            ;;
    esac
    files="$files $HOME/.profile"
    for f in $files; do
        if [ -f "$f" ] && grep -F "$marker" "$f" 2>/dev/null | grep -qF ":$shown:"; then
            continue
        fi
        printf '\n%s\n' "$line" >> "$f"
        say "Added $1 to PATH in $f"
    done
}

# Pins the release key for `agora update`, unless one is pinned already.
pin_release_key() {
    home="${AGORA_HOME:-$HOME/.agora}"
    key_file="$home/release-key"
    if [ -f "$key_file" ]; then
        if [ "$(cat "$key_file")" != "$RELEASE_KEY" ]; then
            say "note: $key_file holds another key than this installer's; it was left as it is"
        fi
        return 0
    fi
    if [ ! -d "$home" ]; then
        (umask 077 && mkdir -p "$home")
    fi
    printf '%s\n' "$RELEASE_KEY" > "$key_file"
    chmod 644 "$key_file"
}

install_main "$@"
