#!/bin/sh
# Abre a tela sem argumentos; com argumentos, é a linha de comando.
PYTHONPATH=/usr/share/ecoreset${PYTHONPATH:+:$PYTHONPATH} exec /usr/bin/python3 -m ecoreset "$@"
