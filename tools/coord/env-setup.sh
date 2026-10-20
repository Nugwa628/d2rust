#!/bin/sh
# Cloud ENVIRONMENT setup script for the D2RUST build loop (paste into the
# environment's settings -> Setup script). Self-contained: it does not rely on
# being run from the repository (the settings run it as pasted text), and it
# never touches the private data repository (a session fetches that itself:
# tools/coord/session-setup.sh, from the forge with $FORGEJO_TOKEN).
# Installs: Bevy build libs, the pinned Rust toolchain, cargo-nextest, Wine
# (32+64 bit) with Xvfb and Vulkan, Windows CPython 3.12.7 (sha256-pinned)
# under Wine in $HOME/winpy with the prefix $HOME/.wine-d2. Idempotent.
# Each step logs "== env-setup: ..." and failures do not stop later steps.
TOOLCHAIN=1.99.0            # keep in step with rust-toolchain.toml
WINPY_VER=3.12.7
NEXTEST_VER=0.9.148       # prebuilt release (1 s); falls back to cargo install (~5 min)
WINPY_SHA=149dd298e0b7a82250ca019471770fff079874088a4e8501ca20922d7df3a6ac
step() { echo "== env-setup: $*"; }
export DEBIAN_FRONTEND=noninteractive

step "apt: Bevy libs, Wine, Xvfb, Vulkan"
dpkg --add-architecture i386 2>/dev/null
apt-get update -qq || step "WARN apt-get update failed"
apt-get install -y -qq --no-install-recommends pkg-config libasound2-dev libudev-dev libwayland-dev \
  libxkbcommon-dev libx11-dev libxcursor-dev libxrandr-dev libxi-dev >/dev/null || step "WARN Bevy libs failed"
apt-get install -y -qq --no-install-recommends wine wine32:i386 wine64 xvfb x11-apps x11-utils \
  imagemagick xdotool mingw-w64 bc mesa-vulkan-drivers libvulkan1 libxkbcommon-x11-0 libx11-xcb1 \
  >/dev/null || step "WARN Wine packages failed"

step "rust toolchain $TOOLCHAIN"
command -v rustup >/dev/null 2>&1 || { curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal >/dev/null; }
. "$HOME/.cargo/env" 2>/dev/null
rustup toolchain list | grep -q "^$TOOLCHAIN" || rustup toolchain install "$TOOLCHAIN" --profile minimal -c clippy -c rustfmt >/dev/null \
  || step "WARN toolchain install failed"

step "cargo-nextest $NEXTEST_VER"
command -v cargo-nextest >/dev/null 2>&1 || curl -sSfL "https://github.com/nextest-rs/nextest/releases/download/cargo-nextest-$NEXTEST_VER/cargo-nextest-$NEXTEST_VER-x86_64-unknown-linux-gnu.tar.gz" \
  | tar zxf - -C "$HOME/.cargo/bin" 2>/dev/null
command -v cargo-nextest >/dev/null 2>&1 || cargo +"$TOOLCHAIN" install cargo-nextest --locked -q || step "WARN nextest failed"

step "Windows Python $WINPY_VER under Wine"
if [ ! -f "$HOME/winpy/tools/python.exe" ] && command -v wine >/dev/null 2>&1; then
  tmp=$(mktemp -d)
  if curl -sSfL -o "$tmp/p.nupkg" "https://api.nuget.org/v3-flatcontainer/python/$WINPY_VER/python.$WINPY_VER.nupkg" \
     && echo "$WINPY_SHA  $tmp/p.nupkg" | sha256sum -c - >/dev/null; then
    rm -rf "$HOME/winpy"; mkdir -p "$HOME/winpy"
    python3 -c "import zipfile,sys; zipfile.ZipFile(sys.argv[1]).extractall(sys.argv[2])" "$tmp/p.nupkg" "$HOME/winpy"
  else step "WARN Windows Python download/check failed"; fi
  rm -rf "$tmp"
fi
if command -v wine >/dev/null 2>&1; then
  export WINEPREFIX="$HOME/.wine-d2" WINEDEBUG=-all WINEDLLOVERRIDES="mscoree,mshtml="
  [ -f "$WINEPREFIX/system.reg" ] || { wineboot -i >/dev/null 2>&1; wineserver -w; }
fi

step "summary: libs=$(pkg-config --exists wayland-client alsa libudev && echo ok || echo MISSING)" \
  "toolchain=$(rustup toolchain list | grep -q "^$TOOLCHAIN" && echo ok || echo MISSING)" \
  "nextest=$(command -v cargo-nextest >/dev/null && echo ok || echo MISSING)" \
  "wine=$(command -v wine >/dev/null && echo ok || echo MISSING)" \
  "winpy=$([ -f "$HOME/winpy/tools/python.exe" ] && echo ok || echo MISSING)" \
  "prefix=$([ -f "$HOME/.wine-d2/system.reg" ] && echo ok || echo MISSING)"
exit 0
