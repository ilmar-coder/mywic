#!/bin/sh
# Gera dist/EcoReset.app, com o libusb dentro (para o acesso USB).
# Precisa de Python 3.8+ com Tk e do libusb: brew install python-tk libusb
set -e
cd "$(dirname "$0")"
LIBUSB="$(brew --prefix)/lib/libusb-1.0.0.dylib"
test -f "$LIBUSB" || { echo "falta o libusb: brew install libusb"; exit 1; }
python3 -m venv .venv
.venv/bin/pip install -q --upgrade pyinstaller
.venv/bin/python -m unittest discover -s . -p "test_*.py"
.venv/bin/pyinstaller --noconfirm --windowed --name EcoReset \
    --osx-bundle-identifier com.ilmar.ecoreset \
    --add-binary "$LIBUSB:." ecoreset_app.py
echo
echo "Pronto: dist/EcoReset.app"
