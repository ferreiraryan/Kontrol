#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(dirname "$(readlink -f "$0")")"
RULES_FILE="${SCRIPT_DIR}/99-knobcontext.rules"

# Localiza o arquivo de serviço systemd (tanto no repositório quanto dentro do AppDir/AppImage)
SERVICE_FILE=""
for candidate in \
    "${SCRIPT_DIR}/knobcontext.service" \
    "${SCRIPT_DIR}/../systemd/knobcontext.service"; do
    if [[ -f "${candidate}" ]]; then
        SERVICE_FILE="${candidate}"
        break
    fi
done

if [[ ! -f "${RULES_FILE}" ]]; then
    echo "Erro: arquivo ${RULES_FILE} não encontrado." >&2
    exit 1
fi

# 1. Verifica se o usuário pertence ao grupo 'input'
if ! id -nG "${USER}" | grep -qw input; then
    echo "AVISO: você não está no grupo 'input'. Rode: sudo usermod -aG input ${USER}"
    echo "Depois faça logout/login para o grupo ter efeito."
fi

# 2. Instala a regra udev (evdev + hidraw)
echo "==> Instalando regra udev em /etc/udev/rules.d/99-knobcontext.rules..."
sudo cp "${RULES_FILE}" /etc/udev/rules.d/99-knobcontext.rules
sudo udevadm control --reload-rules
sudo udevadm trigger
echo "Regra udev instalada. Reconecte o dispositivo knob para aplicar as permissões."

# 3. Instala o executável em ~/.local/bin/knobcontext (se disponível)
INSTALL_BIN="${HOME}/.local/bin/knobcontext"
mkdir -p "${HOME}/.local/bin"

SOURCE_BIN=""
for candidate in \
    "${SCRIPT_DIR}/../../KnobContext-x86_64.AppImage" \
    "${SCRIPT_DIR}/KnobContext-x86_64.AppImage" \
    "${SCRIPT_DIR}/usr/bin/knobcontext" \
    "${SCRIPT_DIR}/../../knobcontext/target/release/knobcontext"; do
    if [[ -f "${candidate}" ]]; then
        SOURCE_BIN="${candidate}"
        break
    fi
done

if [[ -n "${SOURCE_BIN}" ]]; then
    echo "==> Instalando executável em ${INSTALL_BIN} (origem: ${SOURCE_BIN})..."
    cp -f "${SOURCE_BIN}" "${INSTALL_BIN}"
    chmod +x "${INSTALL_BIN}"
elif [[ ! -x "${INSTALL_BIN}" ]]; then
    echo "AVISO: nenhum binário/AppImage encontrado automaticamente."
    echo "Copie KnobContext-x86_64.AppImage para ${INSTALL_BIN} e dê chmod +x antes de iniciar o serviço."
fi

# 4. Instala e habilita o serviço systemd --user
if [[ -n "${SERVICE_FILE}" ]]; then
    USER_SYSTEMD_DIR="${XDG_CONFIG_HOME:-${HOME}/.config}/systemd/user"
    mkdir -p "${USER_SYSTEMD_DIR}"
    echo "==> Instalando serviço systemd --user em ${USER_SYSTEMD_DIR}/knobcontext.service..."
    cp -f "${SERVICE_FILE}" "${USER_SYSTEMD_DIR}/knobcontext.service"
    systemctl --user daemon-reload
    if [[ -x "${INSTALL_BIN}" ]]; then
        systemctl --user enable --now knobcontext.service
        echo "Serviço knobcontext.service habilitado e iniciado."
        echo "Verifique o status com: systemctl --user status knobcontext.service"
        echo "Acompanhe os logs com: journalctl --user -u knobcontext.service -f"
    else
        systemctl --user enable knobcontext.service
        echo "Serviço knobcontext.service habilitado (copie o binário para ${INSTALL_BIN} e rode: systemctl --user start knobcontext.service)."
    fi
else
    echo "AVISO: arquivo knobcontext.service não encontrado; serviço systemd não instalado."
fi
