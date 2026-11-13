#!/usr/bin/env bash

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}   Hebnix Linux Installation Script${NC}"
echo -e "${BLUE}========================================${NC}\n"

# Track if any dependencies are missing
MISSING_DEPS=0
MISSING_OPTIONAL_DEPS=0

# Function to print status
print_status() {
    if [ $1 -eq 0 ]; then
        echo -e "${GREEN}✓${NC} $2"
    else
        echo -e "${RED}✗${NC} $2"
    fi
}

print_warning() {
    echo -e "${YELLOW}⚠${NC} $1"
}

print_info() {
    echo -e "${BLUE}ℹ${NC} $1"
}

# Check for required commands
check_command() {
    if command -v "$1" &> /dev/null; then
        print_status 0 "$1 found"
        return 0
    else
        print_status 1 "$1 not found"
        return 1
    fi
}

# Detect distribution
detect_distro() {
    if [ -f /etc/os-release ]; then
        . /etc/os-release
        echo "$ID"
    else
        echo "unknown"
    fi
}

DISTRO=$(detect_distro)
print_info "Detected distribution: $DISTRO"
echo ""

# Check for Rust
echo -e "${BLUE}[1/6] Checking Rust installation...${NC}"
if check_command rustc && check_command cargo; then
    RUST_VERSION=$(rustc --version | awk '{print $2}')
    print_info "Rust version: $RUST_VERSION"
else
    MISSING_DEPS=1
    print_warning "Rust is required. Install via: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
fi
echo ""

# Check for C compiler
echo -e "${BLUE}[2/6] Checking C compiler...${NC}"
if check_command gcc || check_command clang; then
    if command -v gcc &> /dev/null; then
        GCC_VERSION=$(gcc --version | head -n1)
        print_info "$GCC_VERSION"
    elif command -v clang &> /dev/null; then
        CLANG_VERSION=$(clang --version | head -n1)
        print_info "$CLANG_VERSION"
    fi
else
    MISSING_DEPS=1
fi
echo ""

# Check for pkg-config
echo -e "${BLUE}[3/6] Checking build tools...${NC}"
if ! check_command pkg-config; then
    MISSING_DEPS=1
fi

if ! check_command make; then
    MISSING_DEPS=1
fi

# the Workshop multiplayer helper (sidecar/hebnix-tc) is written in Go
if ! check_command go; then
    MISSING_DEPS=1
    print_warning "Go is required to build the multiplayer helper (Arch: sudo pacman -S go, Debian/Ubuntu: sudo apt install golang)"
fi
echo ""

# Check for system libraries
echo -e "${BLUE}[4/6] Checking system libraries...${NC}"

check_library() {
    if pkg-config --exists "$1" 2>/dev/null; then
        print_status 0 "$1 found"
        return 0
    else
        print_status 1 "$1 not found"
        return 1
    fi
}

# Check GTK3
if ! check_library gtk+-3.0; then
    MISSING_DEPS=1
fi

# Check Wayland
if ! check_library wayland-client; then
    MISSING_DEPS=1
fi

# Check xkbcommon
if ! check_library xkbcommon; then
    MISSING_DEPS=1
fi

# Check for app indicator (varies by distro)
if ! check_library ayatana-appindicator3-0.1 && ! check_library appindicator3-0.1; then
    MISSING_DEPS=1
fi

# Check udev (gamepad hotplug detection)
if ! check_library libudev; then
    MISSING_DEPS=1
fi

# Check alsa (audio playback)
if ! check_library alsa; then
    MISSING_DEPS=1
fi

# Check openssl (websocket/https client)
if ! check_library openssl; then
    MISSING_DEPS=1
fi

# Check libxdo, x11, xtst, xi (window focus tracking + synthetic input)
if ! check_library libxdo; then
    MISSING_DEPS=1
fi
if ! check_library x11; then
    MISSING_DEPS=1
fi
if ! check_library xtst; then
    MISSING_DEPS=1
fi
if ! check_library xi; then
    MISSING_DEPS=1
fi

# Check webkit2gtk/libsoup/gtk-layer-shell (html plugin overlay)
if ! check_library webkit2gtk-4.1; then
    MISSING_DEPS=1
fi
if ! check_library libsoup-3.0; then
    MISSING_DEPS=1
fi
if ! check_library gtk-layer-shell-0; then
    MISSING_DEPS=1
fi
echo ""

# Check for optional dependencies
echo -e "${BLUE}[5/6] Checking optional dependencies...${NC}"

# Check for KDE tools if on KDE
if [ "$XDG_CURRENT_DESKTOP" = "KDE" ] || [ "$DESKTOP_SESSION" = "plasma" ]; then
    if ! check_command kdotool; then
        MISSING_OPTIONAL_DEPS=1
        print_warning "kdotool not found (recommended for KDE/Plasma)"
    fi
    if ! check_command kscreen-doctor; then
        MISSING_OPTIONAL_DEPS=1
        print_warning "kscreen-doctor not found (part of KDE Plasma)"
    fi
fi
echo ""

# Check keyboard/controller reading (/dev/input/event*, needs the `input`
# group) and synthetic input (/dev/uinput, virtual keyboard used by
# hebnix.input.send / hebnix.chat.send -- e.g. the quick-chat plugin).
# Neither is a hard requirement: without them hotkeys/binds/chat-send
# plugins just silently report "not pressed" / fail to type, everything
# else in the app works fine.
echo -e "${BLUE}[6/6] Checking input device access...${NC}"
NEEDS_INPUT_GROUP=0
if groups "$USER" | grep -qw input; then
    print_status 0 "'$USER' is in the 'input' group"
else
    NEEDS_INPUT_GROUP=1
    MISSING_OPTIONAL_DEPS=1
    print_warning "'$USER' is not in the 'input' group (needed to read hotkeys/binds and for synthetic chat-send input)"
fi

NEEDS_UINPUT_RULE=0
if [ -e /dev/uinput ]; then
    if [ -w /dev/uinput ] || { [ "$NEEDS_INPUT_GROUP" -eq 1 ] && stat -c '%G' /dev/uinput 2>/dev/null | grep -qw input; }; then
        print_status 0 "/dev/uinput present and writable"
    else
        NEEDS_UINPUT_RULE=1
        MISSING_OPTIONAL_DEPS=1
        print_warning "/dev/uinput present but not group-'input'-writable ($(stat -c '%G %a' /dev/uinput 2>/dev/null)) -- synthetic input (chat-send plugins) won't work until fixed"
    fi
else
    NEEDS_UINPUT_RULE=1
    MISSING_OPTIONAL_DEPS=1
    print_warning "/dev/uinput not present (uinput kernel module not loaded)"
fi
echo ""

if [ $NEEDS_INPUT_GROUP -eq 1 ] || [ $NEEDS_UINPUT_RULE -eq 1 ]; then
    echo -e "${YELLOW}Synthetic input (chat-send plugins, e.g. quick-chat) needs:${NC}"
    [ $NEEDS_INPUT_GROUP -eq 1 ] && echo "  - your user in the 'input' group"
    [ $NEEDS_UINPUT_RULE -eq 1 ] && echo "  - /dev/uinput loaded and group-'input'-writable"
    echo ""
    read -p "Set this up now? Needs sudo, and a re-login to take effect. [y/N] " -n 1 -r
    echo ""
    if [[ $REPLY =~ ^[Yy]$ ]]; then
        if [ $NEEDS_UINPUT_RULE -eq 1 ]; then
            echo "uinput" | sudo tee /etc/modules-load.d/uinput.conf > /dev/null
            sudo modprobe uinput
            echo 'KERNEL=="uinput", GROUP="input", MODE="0660", OPTIONS+="static_node=uinput"' \
                | sudo tee /etc/udev/rules.d/60-hebnix-uinput.rules > /dev/null
            sudo udevadm control --reload-rules
            sudo udevadm trigger /dev/uinput 2>/dev/null || true
            print_status 0 "uinput module + udev rule installed"
        fi
        if [ $NEEDS_INPUT_GROUP -eq 1 ]; then
            sudo usermod -aG input "$USER"
            print_status 0 "'$USER' added to the 'input' group (re-login or reboot to take effect)"
        fi
    else
        print_info "Skipped. You can do this later -- see README.md for the manual steps."
    fi
fi
echo ""

# Print installation instructions if dependencies are missing
if [ $MISSING_DEPS -eq 1 ]; then
    echo -e "${RED}========================================${NC}"
    echo -e "${RED}   Missing Required Dependencies${NC}"
    echo -e "${RED}========================================${NC}\n"

    case "$DISTRO" in
        arch|manjaro)
            echo -e "${YELLOW}Run the following command to install dependencies:${NC}"
            echo -e "${GREEN}sudo pacman -S --needed base-devel gtk3 libayatana-appindicator wayland libxkbcommon \\
    systemd-libs alsa-lib openssl xdotool libx11 libxtst libxi webkit2gtk-4.1 libsoup3 gtk-layer-shell${NC}"
            echo ""
            echo -e "${YELLOW}For Rust (if not installed):${NC}"
            echo -e "${GREEN}curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh${NC}"
            ;;
        ubuntu|debian|linuxmint|pop)
            echo -e "${YELLOW}Run the following command to install dependencies:${NC}"
            echo -e "${GREEN}sudo apt install build-essential pkg-config libgtk-3-dev libayatana-appindicator3-dev \\
    libwayland-dev libxkbcommon-dev libudev-dev libasound2-dev libssl-dev libxdo-dev \\
    libx11-dev libxtst-dev libxi-dev libwebkit2gtk-4.1-dev libsoup-3.0-dev libgtk-layer-shell-dev${NC}"
            echo ""
            echo -e "${YELLOW}For Rust (if not installed):${NC}"
            echo -e "${GREEN}curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh${NC}"
            ;;
        fedora|rhel|centos)
            echo -e "${YELLOW}Run the following command to install dependencies:${NC}"
            echo -e "${GREEN}sudo dnf install gcc gtk3-devel libappindicator-gtk3-devel wayland-devel libxkbcommon-devel \\
    systemd-devel alsa-lib-devel openssl-devel libxdo-devel libX11-devel libXtst-devel \\
    libXi-devel webkit2gtk4.1-devel libsoup3-devel gtk-layer-shell-devel${NC}"
            echo ""
            echo -e "${YELLOW}For Rust (if not installed):${NC}"
            echo -e "${GREEN}curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh${NC}"
            ;;
        *)
            echo -e "${YELLOW}Please install the following dependencies for your distribution:${NC}"
            echo "  - Rust (stable) via rustup"
            echo "  - C compiler (gcc or clang)"
            echo "  - GTK3 development libraries"
            echo "  - Wayland client development libraries"
            echo "  - libxkbcommon development libraries"
            echo "  - libayatana-appindicator or libappindicator development libraries"
            echo "  - libudev development libraries (gamepad support)"
            echo "  - ALSA development libraries (audio)"
            echo "  - OpenSSL development libraries"
            echo "  - libxdo, libX11, libXtst, libXi development libraries (window focus + synthetic input)"
            echo "  - webkit2gtk-4.1, libsoup-3.0, gtk-layer-shell development libraries (html plugin overlay)"
            ;;
    esac
    echo ""
    exit 1
fi

# Print optional dependency info
if [ $MISSING_OPTIONAL_DEPS -eq 1 ]; then
    echo -e "${YELLOW}========================================${NC}"
    echo -e "${YELLOW}   Optional Dependencies Missing${NC}"
    echo -e "${YELLOW}========================================${NC}\n"
    if [ "$XDG_CURRENT_DESKTOP" = "KDE" ] || [ "$DESKTOP_SESSION" = "plasma" ]; then
        print_info "kdotool: For window focus tracking on KDE/Plasma"
        print_info "Install from AUR: yay -S kdotool-bin"
        echo ""
    fi
fi

# All dependencies met, proceed with build
echo -e "${GREEN}========================================${NC}"
echo -e "${GREEN}   All Required Dependencies Met${NC}"
echo -e "${GREEN}========================================${NC}\n"

echo -e "${BLUE}Building and installing Hebnix...${NC}"
echo ""

cd "$SCRIPT_DIR"

# Canonical build+install path (same one GitHub Actions and the AUR
# packages use) -- installs the `hebnix` command, .desktop file, and icon
# under ~/.local, no sudo needed. Plugins/themes/config are managed by the
# app itself at $XDG_CONFIG_HOME/hebnix (~/.config/hebnix by default).
if make install; then
    echo ""
    echo -e "${GREEN}========================================${NC}"
    echo -e "${GREEN}   Install Successful!${NC}"
    echo -e "${GREEN}========================================${NC}\n"

    if command -v update-desktop-database &> /dev/null; then
        update-desktop-database "$HOME/.local/share/applications" 2>/dev/null
    fi

    print_info "Installed to: $HOME/.local/bin/hebnix"
    print_info "Config, plugins and themes live in: \${XDG_CONFIG_HOME:-\$HOME/.config}/hebnix"
    echo ""
    print_info "To run Hebnix:"
    echo -e "  ${GREEN}hebnix${NC}  (make sure ~/.local/bin is on your PATH)"
    echo -e "  ${GREEN}Or launch it from your application menu${NC}"
    echo ""
    print_info "On first run, an empty plugins/ folder will be created."
    print_info "Install plugins via the app's Plugins tab or clone them manually."
    echo ""

    if [ $MISSING_OPTIONAL_DEPS -eq 1 ]; then
        print_warning "Remember to install optional dependencies for full functionality."
    fi
else
    echo ""
    echo -e "${RED}========================================${NC}"
    echo -e "${RED}   Build Failed${NC}"
    echo -e "${RED}========================================${NC}\n"
    exit 1
fi
