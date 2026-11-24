#!/bin/bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$(readlink -f "$0")")/.." && pwd)"
APPIMAGE_ASSETS="${ROOT_DIR}/packaging/appimage"
SYSTEMD_ASSETS="${ROOT_DIR}/packaging/systemd"
APPDIR="${ROOT_DIR}/packaging/KnobContext.AppDir"
OUTPUT="${ROOT_DIR}/KnobContext-x86_64.AppImage"

export PATH="${HOME}/.local/bin:${HOME}/.cargo/bin:${PATH}"

echo "==> Compilando KnobContext em modo release..."
(cd "${ROOT_DIR}/knobcontext" && cargo build --release --bin knobcontext)

BIN_PATH="${ROOT_DIR}/knobcontext/target/release/knobcontext"
if [[ ! -x "${BIN_PATH}" ]]; then
    echo "Erro: binário release não encontrado em ${BIN_PATH}" >&2
    exit 1
fi

echo "==> Montando AppDir em ${APPDIR}..."
rm -rf "${APPDIR}"
mkdir -p "${APPDIR}/usr/bin" "${APPDIR}/usr/lib" "${APPDIR}/usr/share/applications" "${APPDIR}/usr/share/icons/hicolor/256x256/apps"

cp "${BIN_PATH}" "${APPDIR}/usr/bin/knobcontext"
cp "${APPIMAGE_ASSETS}/AppRun" "${APPDIR}/AppRun"
cp "${APPIMAGE_ASSETS}/knobcontext.desktop" "${APPDIR}/knobcontext.desktop"
cp "${APPIMAGE_ASSETS}/knobcontext.desktop" "${APPDIR}/usr/share/applications/knobcontext.desktop"
cp "${APPIMAGE_ASSETS}/knobcontext.png" "${APPDIR}/knobcontext.png"
cp "${APPIMAGE_ASSETS}/knobcontext.png" "${APPDIR}/usr/share/icons/hicolor/256x256/apps/knobcontext.png"
cp "${APPIMAGE_ASSETS}/99-knobcontext.rules" "${APPDIR}/99-knobcontext.rules"
cp "${APPIMAGE_ASSETS}/install.sh" "${APPDIR}/install.sh"
if [[ -f "${SYSTEMD_ASSETS}/knobcontext.service" ]]; then
    cp "${SYSTEMD_ASSETS}/knobcontext.service" "${APPDIR}/knobcontext.service"
fi

chmod +x "${APPDIR}/AppRun" "${APPDIR}/install.sh" "${APPDIR}/usr/bin/knobcontext"

if command -v linuxdeploy >/dev/null 2>&1; then
    echo "==> Gerando AppImage via linuxdeploy..."
    ARCH=x86_64 linuxdeploy \
        --appdir "${APPDIR}" \
        --desktop-file "${APPDIR}/knobcontext.desktop" \
        --icon-file "${APPDIR}/knobcontext.png" \
        --output appimage
    mv -f "${ROOT_DIR}"/KnobContext*.AppImage "${OUTPUT}" 2>/dev/null || true
elif command -v appimagetool >/dev/null 2>&1; then
    echo "==> Gerando AppImage via appimagetool..."
    ARCH=x86_64 appimagetool "${APPDIR}" "${OUTPUT}"
else
    echo ""
    echo "AVISO: nem 'linuxdeploy' nem 'appimagetool' foram encontrados no PATH." >&2
    echo "O AppDir foi montado com sucesso em: ${APPDIR}" >&2
    echo "Para gerar o arquivo .AppImage no Arch Linux, instale appimagetool (AUR: appimagetool-bin) ou baixe o binário oficial:" >&2
    echo "  curl -LO https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage" >&2
    echo "  chmod +x appimagetool-x86_64.AppImage" >&2
    echo "  ARCH=x86_64 ./appimagetool-x86_64.AppImage \"${APPDIR}\" \"${OUTPUT}\"" >&2
    exit 1
fi

chmod +x "${OUTPUT}"
echo "==> AppImage gerado com sucesso: ${OUTPUT}"
