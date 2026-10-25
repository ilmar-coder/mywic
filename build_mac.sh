#!/bin/sh
# Gera dist/EcoReset.app. Precisa de Python 3.8+ com Tk (brew install python-tk).
set -e
cd "$(dirname "$0")"
python3 -m venv .venv
.venv/bin/pip install -q --upgrade pyinstaller
.venv/bin/python -m unittest discover -s . -p "test_*.py"
.venv/bin/pyinstaller --noconfirm --windowed --name EcoReset \
    --osx-bundle-identifier com.ilmar.ecoreset ecoreset_app.py
echo
echo "Pronto: dist/EcoReset.app"
