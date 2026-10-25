"""Parâmetros por modelo: chaves e endereços de EEPROM.

São fatos de hardware publicados pela comunidade (projeto epson_print_conf,
de Ircama). Cada contador é uma lista de endereços do byte menos para o mais
significativo; o valor bruto dividido por `divider` dá a porcentagem.
"""

_L4160_RESET = {48: 0, 49: 0, 47: 0, 52: 0, 53: 0, 54: 94, 50: 0, 51: 0, 55: 94, 28: 0}
_L355_RESET = {24: 0, 25: 0, 30: 0, 28: 0, 29: 0, 46: 94}

MODELS = {
    "L4160": {
        "aliases": ["L4150", "L4152", "L4154", "L4156", "L4158", "L4162",
                    "L4164", "L4166", "L4168"],
        "read_key": (73, 8), "write_key": b"Arantifo",
        "main_waste": {"oids": [48, 49, 47], "divider": 109.13},
        "borderless_waste": {"oids": [50, 51, 47], "divider": 16.31},
        "raw_waste_reset": _L4160_RESET,
    },
    "L3150": {
        "aliases": ["L3151", "L3160", "L3166", "L3168"],
        "read_key": (151, 7), "write_key": b"Maribaya",
        "main_waste": {"oids": [48, 49, 47], "divider": 63.46},
        "borderless_waste": {"oids": [50, 51, 47], "divider": 34.16},
        "raw_waste_reset": _L4160_RESET,
    },
    "L355": {
        "aliases": [],
        "read_key": (65, 9), "write_key": b"Wakatobi",
        "main_waste": {"oids": [24, 25, 30], "divider": 65.0},
        "raw_waste_reset": _L355_RESET,
    },
    "L405": {
        "aliases": [],
        "read_key": (149, 3), "write_key": b"Maninjau",
        "main_waste": {"oids": [24, 25, 30], "divider": 62.07},
        "raw_waste_reset": _L355_RESET,
    },
    "L365": {
        "aliases": [],
        "read_key": (130, 2), "write_key": b"Gerbera*",
        "main_waste": {"oids": [24, 25], "divider": 62.07},
        "raw_waste_reset": _L355_RESET,
    },
    "L366": {
        "aliases": [],
        "read_key": (130, 2), "write_key": b"Gerbera*",
        "main_waste": {"oids": [24, 25, 30], "divider": 62.0625},
        "borderless_waste": {"oids": [26, 27, 34], "divider": 24.20},
    },
    "L3060": {
        "aliases": [],
        "read_key": (149, 3), "write_key": b"Maninjau",
        "main_waste": {"oids": [24, 25, 30], "divider": 62.07},
        "borderless_waste": {"oids": [26, 27, 34], "divider": 24.2},
    },
    "ET-2600": {
        "aliases": ["L395"],
        "read_key": (16, 8), "write_key": b"Sinabung",
        "main_waste": {"oids": [24, 25, 30], "divider": 62.06},
    },
    "ET-2700": {
        "aliases": ["ET-2701", "ET-2703", "ET-2705"],
        "read_key": (73, 8), "write_key": b"Arantifo",
        "main_waste": {"oids": [48, 49, 47], "divider": 109.13},
        "borderless_waste": {"oids": [50, 51, 47], "divider": 16.31},
        "raw_waste_reset": _L4160_RESET,
    },
    "ET-2720": {
        "aliases": ["ET-2714", "ET-2721", "ET-2723", "ET-2725"],
        "read_key": (151, 7), "write_key": b"Maribaya",
        "main_waste": {"oids": [48, 49, 47], "divider": 63.45},
        "borderless_waste": {"oids": [50, 51, 47], "divider": 34.15},
        "raw_waste_reset": _L4160_RESET,
    },
    "ET-2750": {
        "aliases": ["ET-2751", "ET-2756", "ET-2760"],
        "read_key": (73, 8), "write_key": b"Arantifo",
        "main_waste": {"oids": [48, 49, 47], "divider": 109.13},
        "borderless_waste": {"oids": [50, 51, 47], "divider": 16.31},
        "raw_waste_reset": _L4160_RESET,
    },
    "ET-2812": {
        # Família L3250. Muitos firmwares bloqueiam a EEPROM por SNMP; nesse
        # caso só o reset temporário (comando rw) funciona pela rede.
        "aliases": ["ET-2810", "ET-2811", "ET-2813", "ET-2814", "ET-2815",
                    "ET-2816", "ET-2818", "L3250", "L3251", "L3256"],
        "read_key": (74, 54), "write_key": b"Maribaya",
        "main_waste": {"oids": [48, 49, 47], "divider": 63.46},
        "borderless_waste": {"oids": [50, 51, 47], "divider": 34.16},
        "third_waste": {"oids": [252, 253, 254], "divider": 13.0},
        "raw_waste_reset": {48: 0, 49: 0, 47: 0, 52: 0, 53: 0, 54: 94, 50: 0,
                            51: 0, 55: 94, 28: 0, 252: 0, 253: 0, 254: 0, 255: 94},
        # Depois de zerar, limpar o bit 0 do endereço 0x100 tira o erro.
        "waste_reset_commit": {0x100: 0xFE},
        "may_be_locked": True,
    },
    "ET-2800": {
        "aliases": ["ET-2801", "ET-2803", "ET-2805"],
        "read_key": (74, 54), "write_key": b"Maribaya",
        "main_waste": {"oids": [48, 49, 47], "divider": 63.46},
        "borderless_waste": {"oids": [50, 51, 47], "divider": 34.16},
        "third_waste": {"oids": [252, 253, 254], "divider": 13.0},
        "raw_waste_reset": {48: 0, 49: 0, 47: 0, 52: 0, 53: 0, 54: 94, 50: 0,
                            51: 0, 55: 94, 28: 0, 252: 0, 253: 0, 254: 0, 255: 94},
    },
    "ET-4700": {
        "aliases": [],
        "read_key": (151, 7), "write_key": b"Maribaya",
        "main_waste": {"oids": [48, 49, 47], "divider": 63.46},
        "borderless_waste": {"oids": [50, 51, 47], "divider": 34.16},
        "raw_waste_reset": _L4160_RESET,
    },
}


def model_names():
    return sorted(MODELS)


def find_model(text):
    """Acha o modelo num texto como 'EPSON L3150 Series'. Devolve o nome ou None."""
    tokens = text.upper().replace("EPSON", " ").replace("SERIES", " ").split()
    for name, cfg in MODELS.items():
        for candidate in [name] + cfg["aliases"]:
            if candidate in tokens:
                return name
    return None
