#!/usr/bin/env bash
# Build zellij-logging, install it under ~/.config/zellij/plugins/, and print
# the next steps for verifying it inside a Zellij session. We intentionally
# do not auto-launch Zellij: the user's shell, layout, and key bindings are
# session-specific and easy to clobber.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WASM_NAME="zellij_logging.wasm"
TARGET_DIR="${REPO_ROOT}/target/wasm32-wasip1/release"
PLUGIN_DIR="${HOME}/.config/zellij/plugins"

echo "==> Building zellij-logging for wasm32-wasip1..."
( cd "${REPO_ROOT}" && cargo build --release --target wasm32-wasip1 )

if [[ ! -f "${TARGET_DIR}/${WASM_NAME}" ]]; then
    echo "ERROR: expected ${TARGET_DIR}/${WASM_NAME} after build" >&2
    exit 1
fi

mkdir -p "${PLUGIN_DIR}"
echo "==> Copying wasm to ${PLUGIN_DIR}/${WASM_NAME}"
cp "${TARGET_DIR}/${WASM_NAME}" "${PLUGIN_DIR}/${WASM_NAME}"

echo
echo "==> Done. Next steps:"
echo
echo "1. Make sure ~/.config/zellij/config.kdl loads the plugin and binds keys."
echo "   See ${REPO_ROOT}/examples/config.kdl for a drop-in fragment."
echo
echo "2. Launch Zellij from a directory you want logs to live under, e.g."
echo "     cd ~ && zellij"
echo "   With the default output_dir, logs will go to ~/zellij-logs/."
echo
echo "3. In Zellij, focus a terminal pane and press Ctrl+Shift+P to start"
echo "   continuous logging. Run a few commands. Press Ctrl+Shift+P again to"
echo "   stop. Tail the log:"
echo "     find ~/zellij-logs -type f -name '*.log' -printf '%T@ %p\n' \\"
echo "       | sort -rn | head -1 | cut -d' ' -f2- | xargs -r tail -f"
echo
echo "4. Try Alt+P (visible snapshot) and Alt+Shift+P (full scrollback) on a"
echo "   pane that has scrollback. Both produce one-shot files alongside the"
echo "   continuous logs, with .visible.log / .full.log suffixes."
echo
echo "If you don't see any logs:"
echo "  - Approve the ReadPaneContents permission when Zellij prompts."
echo "  - Open the plugin pane in floating mode to see status messages, e.g."
echo "      zellij action launch-or-focus-plugin --floating zellij-logging"
