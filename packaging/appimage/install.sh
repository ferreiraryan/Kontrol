#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(dirname "$(readlink -f "$0")")"
RULES_FILE="${SCRIPT_DIR}/99-knobcontext.rules"

if [[ ! -f "${RULES_FILE}" ]]; then
    echo "Erro: arquivo ${RULES_FILE} não encontrado." >&2
    exit 1
fi

sudo cp "${RULES_FILE}" /etc/udev/rules.d/99-knobcontext.rules
sudo udevadm control --reload-rules
sudo udevadm trigger

echo "Regra udev instalada em /etc/udev/rules.d/99-knobcontext.rules."
echo "Reconecte o dispositivo knob (e garanta que seu usuário está no grupo 'input')."
