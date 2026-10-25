"""Base de erros: o que cada código significa e como resolver.

Epson: códigos do status "@BDC ST2" (tipo 0x02 = erro, 0x01 = estado, 0x04 =
avisos), conforme a tabela pública do epson_print_conf.

Canon: "support codes" da lista oficial do manual da G2060 series
(ij.manual.canon, "List of Support Code for Error"), resumidos em português.
Os códigos de família 5xxx/6xxx/7xxx são parecidos em toda a linha MegaTank.

Cada entrada: (título, causa, correção, precisa_assistência).
"""

from collections import namedtuple

Problem = namedtuple("Problem", "brand code title cause fix service")

CANON_SOURCE = ("https://ij.manual.canon/ij/webmanual/ErrorCode/G2060%20series/"
                "EN/ERR/err_contents0100.html")

_RESTART = ("Desligue a impressora, tire da tomada, ligue de novo. Se o código "
            "voltar, leve à assistência técnica (não incline a impressora: a tinta vaza).")
_REPAIR = "Desligue, tire da tomada e leve à assistência técnica Canon."
_REINSTALL_HEAD = ("Feche a válvula de tinta (alavanca para a esquerda), abra a trava "
                   "das cabeças, retire e recoloque as duas cabeças de impressão "
                   "sem tocar nos contatos dourados nem nos bicos.")

EPSON_STATUS = {
    0x00: "Erro",
    0x01: "Imprimindo auto-teste",
    0x02: "Ocupada",
    0x03: "Aguardando",
    0x04: "Pronta",
    0x05: "Pausada",
    0x07: "Limpando",
    0x08: "Não inicializada (saída de fábrica)",
    0x0A: "Desligando",
    0x0F: "Teste de bicos",
    0x11: "Carregando tinta",
}

EPSON_ERRORS = {
    0x00: ("Erro fatal", "Falha interna (sensor, motor ou placa).",
           "Desligue e ligue. Verifique se nada trava o carro de impressão. "
           "Se continuar, assistência técnica.", True),
    0x01: ("Outra interface em uso", "A impressora está ocupada por outra conexão.",
           "Espere o trabalho da outra conexão terminar.", False),
    0x02: ("Tampa aberta", "A tampa ou o scanner está aberto.",
           "Feche a tampa/scanner até encaixar.", False),
    0x04: ("Papel enroscado", "Papel preso no caminho.",
           "Desligue, retire o papel com cuidado pela frente e por trás, ligue de novo.",
           False),
    0x05: ("Sem tinta", "Um dos tanques está abaixo do mínimo.",
           "Complete o tanque indicado e, se pedido, confirme a recarga no painel.", False),
    0x06: ("Sem papel", "Não há papel ou ele não foi puxado.",
           "Coloque papel e ajuste as guias. Se já tem papel, limpe o rolete.", False),
    0x0C: ("Papel incompatível", "Tamanho, tipo ou caminho do papel não bate com o trabalho.",
           "Confira o tamanho/tipo configurado no driver e o papel carregado.", False),
    0x10: ("Almofada de tinta cheia",
           "O contador da almofada (waste ink pad) chegou ao limite.",
           "Zere o contador em 'Almofada de tinta'. Se a almofada estiver encharcada, "
           "troque-a ou instale reservatório externo antes, senão vaza tinta.", False),
    0x11: ("Aguardando volta do papel", "Papel na posição de corte.",
           "Aguarde ou pressione o botão de papel.", False),
    0x12: ("Alimentação dupla", "Mais de uma folha entrou de uma vez.",
           "Ventile o papel e recoloque menos folhas.", False),
    0x1A: ("Tampa do cartucho aberta", "Tampa dos tanques/cartuchos aberta.",
           "Feche a tampa.", False),
    0x1C: ("Erro do cortador", "Falha no cortador.", "Assistência técnica.", True),
    0x1D: ("Cortador travado", "Papel preso no cortador.",
           "Retire o papel e tente de novo.", False),
    0x22: ("Caixa de manutenção ausente", "A caixa de manutenção não foi encontrada.",
           "Instale ou reencaixe a caixa de manutenção.", False),
    0x25: ("Tampa traseira aberta", "Tampa traseira aberta.", "Feche a tampa traseira.",
           False),
    0x29: ("Bandeja de CD fora", "Bandeja de CD/DVD fora do lugar.",
           "Recoloque a bandeja.", False),
    0x2A: ("Erro no cartão de memória", "Falha ao ler o cartão.",
           "Retire e recoloque o cartão.", False),
    0x2B: ("Tampa da bandeja aberta", "Tampa da bandeja aberta.", "Feche a tampa.", False),
    0x2C: ("Transbordo de tinta no cartucho", "Excesso de tinta detectado.",
           "Verifique vazamentos nos tanques e mangueiras.", True),
    0x2F: ("Bateria com tensão anormal", "Bateria com problema.", "Troque a bateria.",
           False),
    0x30: ("Bateria com temperatura anormal", "Bateria quente ou fria demais.",
           "Espere a bateria voltar à temperatura ambiente.", False),
    0x31: ("Bateria vazia", "Bateria descarregada.", "Carregue a bateria.", False),
    0x33: ("Carga inicial impossível", "Não deu para carregar a tinta inicial.",
           "Complete todos os tanques e repita a carga inicial.", False),
    0x36: ("Tampa da caixa de manutenção aberta", "Tampa da caixa aberta.",
           "Feche a tampa.", False),
    0x37: ("Scanner ou tampa frontal aberta", "Scanner ou tampa frontal aberta.",
           "Feche o scanner/tampa.", False),
    0x41: ("Manutenção necessária", "A impressora pede manutenção (em geral, almofada).",
           "Leia os contadores. Se estiverem altos, zere-os e cuide da almofada.", False),
    0x47: ("Impressão desativada", "A impressão foi bloqueada pela impressora.",
           "Desligue e ligue. Verifique erros anteriores.", True),
    0x4A: ("Caixa de manutenção quase cheia", "A caixa de manutenção está quase cheia.",
           "Providencie uma caixa nova.", False),
    0x4B: ("Driver incompatível", "O driver não corresponde à impressora.",
           "Instale o driver correto do modelo.", False),
}

EPSON_WARNINGS = {
    0x10: "Tinta baixa (preto ou amarelo)",
    0x11: "Tinta baixa (magenta)",
    0x12: "Tinta baixa (amarelo ou ciano)",
    0x13: "Tinta baixa (ciano ou preto fosco)",
    0x14: "Tinta baixa (preto foto)",
    0x15: "Tinta baixa (vermelho)",
    0x16: "Tinta baixa (azul)",
    0x17: "Tinta baixa (otimizador de brilho)",
    0x44: "Modo só preto",
    0x51: "Limpeza desativada (ciano)",
    0x52: "Limpeza desativada (magenta)",
    0x53: "Limpeza desativada (amarelo)",
    0x54: "Limpeza desativada (preto)",
}

CANON_CODES = {
    "1000": ("Sem papel", "Bandeja traseira sem papel ou papel mal colocado.",
             "Coloque papel, ajuste as guias, feche a tampa do alimentador e aperte OK.",
             False),
    "1070": ("Rolete sujo", "Os roletes de alimentação estão sujos.",
             "Cancele com Stop e faça a limpeza dos roletes (painel ou driver).", False),
    "1200": ("Tampa aberta", "Unidade de scanner/tampa aberta.",
             "Feche o scanner/tampa (inclusive depois de completar tinta).", False),
    "1215": ("Troca de cabeça em andamento", "A troca da cabeça não terminou.",
             "Termine a troca. Se desistir, feche a válvula de tinta (alavanca para a "
             "direita) e feche a tampa.", False),
    "1300": ("Papel enroscado", "Papel preso ao puxar da bandeja traseira.",
             "Puxe o papel devagar com as duas mãos pela saída ou pela bandeja. Se não "
             "sair, desligue e ligue: ele pode ser ejetado sozinho.", False),
    "1366": ("Válvula de tinta fechada", "A alavanca da válvula de tinta está fechada.",
             "Abra a tampa e gire a alavanca da válvula para a direita até o fim.", False),
    "1368": ("Caixa de manutenção quase cheia (Ink Flush)",
             "Não dá para fazer Ink Flush com a caixa quase cheia.",
             "Desligue e troque a caixa de manutenção.", False),
    "1431": ("Cabeça não reconhecida", "Cabeça mal instalada ou incompatível.",
             _REINSTALL_HEAD, False),
    "1432": ("Cabeça não reconhecida", "Cabeça mal instalada ou incompatível.",
             _REINSTALL_HEAD, False),
    "1470": ("Cabeça mal instalada", "A cabeça não está travada.",
             "Empurre firme a trava das cabeças e os botões de junção até o fim. Tire "
             "a etiqueta laranja e a fita protetora de cabeças novas.", False),
    "1471": ("Cabeça não reconhecida", "Cabeça mal instalada ou incompatível.",
             _REINSTALL_HEAD, False),
    "1472": ("Cabeça não reconhecida", "Cabeça mal instalada ou incompatível.",
             _REINSTALL_HEAD, False),
    "1473": ("Cabeça não reconhecida", "Cabeça mal instalada ou incompatível.",
             _REINSTALL_HEAD, False),
    "1476": ("Cabeça incompatível", "As cabeças instaladas não servem neste modelo.",
             "Instale cabeças compatíveis com a impressora.", False),
    "1496": ("Caixa de manutenção não reconhecida", "Caixa mal encaixada ou incompatível.",
             "Desligue, retire e reencaixe a caixa de manutenção.", False),
    "15A0": ("Tanques vazios na instalação", "Não há tinta nos tanques.",
             "Encha os tanques com os frascos que vieram com a impressora, feche as "
             "tampas e confirme no painel.", False),
    "15A1": ("Instalação incompleta", "A configuração inicial não terminou.",
             "Siga a configuração no visor da impressora e depois no computador.", False),
    "1640": ("Tinta no limite", "Algum tanque chegou à linha mínima.",
             "Confira os tanques e complete a tinta. Dá para desativar o aviso, mas "
             "imprimir sem tinta pode danificar a cabeça.", False),
    "1723": ("Caixa de manutenção ausente", "A caixa de manutenção não está instalada.",
             "Desligue e instale a caixa de manutenção.", False),
    "1724": ("Caixa de manutenção usada em outra impressora",
             "Caixas já instaladas em outra impressora não são aceitas.",
             "Desligue e coloque uma caixa de manutenção nova.", False),
    "1725": ("Caixa de manutenção não reconhecida", "Caixa mal encaixada ou incompatível.",
             "Desligue, retire e reencaixe a caixa de manutenção.", False),
    "1726": ("Caixa de manutenção cheia",
             "A caixa que recolhe a tinta descartada encheu. Nesta linha não existe "
             "reset: a peça é trocável.",
             "Desligue e troque a caixa de manutenção.", False),
    "1727": ("Caixa de manutenção quase cheia", "Logo não será possível imprimir.",
             "Aperte OK para continuar e providencie uma caixa nova.", False),
    "1890": ("Fita de transporte", "Fita ou proteção ainda presa no suporte da cabeça.",
             "Abra a tampa e retire toda fita/proteção do suporte da cabeça.", False),
    "1892": ("Trava do carro", "A trava de transporte do carro ainda está colocada.",
             "Abra a tampa, puxe a trava para cima e guarde-a no encaixe ao lado.", False),
    "2114": ("Papel não confere", "O papel configurado não bate com o registrado na bandeja.",
             "Ajuste o tipo/tamanho no driver ou registre o papel certo na impressora.", False),
    "2123": ("Registro do papel incompleto", "Falta confirmar o papel da bandeja traseira.",
             "Confirme com OK se o papel bate, ou registre o papel correto.", False),
    "2200": ("Peça interna no fim da vida", "Uma peça interna precisará ser trocada em breve.",
             "Aperte OK para continuar e agende reparo na assistência Canon.", True),
    "2700": ("Tempo esgotado na cópia", "Erro durante a cópia e passou tempo demais.",
             "Aperte OK e copie de novo.", False),
    "2900": ("Falha ao ler folha de alinhamento", "O scanner não leu a folha de alinhamento.",
             "Posicione a folha certa no vidro (limpo), confira os bicos e refaça o "
             "alinhamento; se falhar, faça o manual.", False),
    "2901": ("Aguardando ler alinhamento", "A folha de alinhamento foi impressa.",
             "Coloque-a virada para baixo no vidro, alinhada à marca, e aperte Color "
             "ou Black.", False),
    "3252": ("Caixa de manutenção quase cheia", "Logo não será possível imprimir.",
             "Aperte OK e providencie uma caixa nova.", False),
    "3402": ("Memória cheia (fila de cópias)", "Não cabe mais cópia na fila.",
             "Aperte OK, espere e refaça a partir do original que deu erro.", False),
    "3403": ("Memória cheia (fila de cópias)", "Não cabe mais cópia na fila.",
             "Aperte OK, espere e recomece do início.", False),
    "3405": ("Memória cheia (cópia)", "Memória insuficiente para a cópia.",
             "Aperte OK, espere e tente de novo; se continuar, baixe a qualidade.", False),
    "3407": ("Próxima folha", "A impressora espera a próxima folha.",
             "Coloque a próxima folha e aperte OK (ou Color/Black para seguir sem ela).",
             False),
    "3410": ("Escaneando", "Não dá para enfileirar cópia durante a leitura.",
             "Espere terminar e tente de novo.", False),
    "3411": ("Pronta para enfileirar cópia", "Pode enfileirar outra cópia.",
             "Coloque o original e aperte Color ou Black.", False),
    "3412": ("Falha ao escanear original", "Original ausente ou mal posicionado.",
             "Posicione o original no vidro, na orientação certa, e tente de novo.", False),
    "3413": ("Proteção contra arranhões ligada", "A opção contra arranhões está ativa.",
             "Aperte OK para seguir ou desative em Configurações > Impressão.", False),
    "3438": ("Original esquecido no vidro", "Ficou um original no vidro.",
             "Retire o original e aperte OK.", False),
    "3442": ("Frente e verso manual", "A primeira face foi impressa.",
             "Recoloque o papel com a face impressa para baixo, sem girar, e aperte OK.",
             False),
    "3443": ("Frente e verso manual", "A primeira face foi impressa.",
             "Gire o papel 180°, coloque com a face impressa para baixo e aperte OK.", False),
    "3444": ("Frente e verso manual", "A primeira face foi impressa.",
             "Gire o papel 180°, coloque com a face impressa para baixo e aperte OK.", False),
    "3445": ("Frente e verso manual", "A primeira face foi impressa.",
             "Recoloque o papel com a face impressa para baixo, sem girar, e aperte OK.",
             False),
    "3449": ("Alinhamento necessário", "A cabeça precisa ser alinhada.",
             "Aperte OK e faça o alinhamento da cabeça se a impressão sair desalinhada.",
             False),
    "4103": ("Configuração inválida", "Não dá para imprimir com essas configurações.",
             "Cancele com Stop, mude as configurações e imprima de novo.", False),
    "4104": ("Tamanho de papel inválido", "Tamanho de papel não suportado.",
             "Cancele com Stop e escolha um tamanho suportado.", False),
    "5011": ("Erro da impressora", "Falha interna.", _RESTART, True),
    "5012": ("Erro da impressora", "Falha interna.", _RESTART, True),
    "5050": ("Erro da impressora", "Falha interna.", _RESTART, True),
    "5100": ("Erro do carro", "O movimento do carro de impressão está bloqueado.",
             "Cancele, desligue e retire fitas, papel ou objetos do caminho do carro, "
             "sem tocar no filme transparente, na correia branca nem nos tubos. Ligue de "
             "novo; se continuar, assistência.", True),
    "5200": ("Erro de temperatura da cabeça", "A cabeça esquentou demais (falta de tinta).",
             "Nesta ordem: complete a tinta, desligue e religue, faça limpeza e depois "
             "Ink Flush. Se voltar, troque as cabeças; se ainda voltar, assistência.", True),
    "5205": ("Erro da impressora", "Falha interna.", _RESTART, True),
    "5206": ("Erro da impressora", "Falha interna.", _RESTART, True),
    "5400": ("Erro da impressora", "Falha interna.", _RESTART, True),
    "5700": ("Erro da impressora", "Falha interna.", _RESTART, True),
    "5C30": ("Erro que exige reparo", "Falha que só a assistência resolve.", _REPAIR, True),
    "6000": ("Erro de alimentação", "Falha no mecanismo de papel (muitas vezes papel preso).",
             "Retire papel enroscado, desligue, tire da tomada e ligue. Se continuar, "
             "assistência.", True),
    "6500": ("Erro da impressora", "Falha interna.", _RESTART, True),
    "6800": ("Erro da impressora", "Falha interna.", _RESTART, True),
    "6801": ("Erro da impressora", "Falha interna.", _RESTART, True),
    "6D01": ("Erro da impressora", "Falha interna.", _RESTART, True),
    "7500": ("Erro que exige reparo", "Falha que só a assistência resolve.", _REPAIR, True),
    "7600": ("Erro que exige reparo", "Falha que só a assistência resolve.", _REPAIR, True),
    "7700": ("Erro que exige reparo", "Falha que só a assistência resolve.", _REPAIR, True),
    "7800": ("Erro que exige reparo", "Falha que só a assistência resolve.", _REPAIR, True),
    "B20B": ("Erro que exige reparo", "Falha que só a assistência resolve.", _REPAIR, True),
    # Linhas sem caixa de manutenção (G2010, G3110...): não estão no manual da
    # G2060; incluídos pelo significado conhecido em toda a linha PIXMA.
    "5B00": ("Almofada de tinta cheia",
             "O absorvedor de tinta (almofada) chegou ao limite. Comum em modelos sem "
             "caixa de manutenção.",
             "Exige troca da almofada e reset pelo modo de serviço da Canon (ferramenta "
             "proprietária). Procure a assistência técnica.", True),
    "5B01": ("Almofada de tinta cheia", "Variação do 5B00.",
             "Mesma ação do 5B00: assistência técnica.", True),
    "5B02": ("Almofada de tinta cheia", "Variação do 5B00.",
             "Mesma ação do 5B00: assistência técnica.", True),
    "1700": ("Almofada de tinta quase cheia", "O absorvedor está perto do limite.",
             "Aperte OK para continuar e planeje a troca da almofada na assistência.",
             False),
}
for _code in ("6930", "6931", "6932", "6933", "6936", "6937", "6938", "6940", "6941",
              "6942", "6943", "6944", "6945", "6946"):
    CANON_CODES[_code] = ("Erro da impressora", "Falha interna.", _RESTART, True)


def explain_epson_error(code):
    title, cause, fix, service = EPSON_ERRORS.get(
        code, ("Erro desconhecido (0x%02X)" % code, "Código não catalogado.",
               "Desligue e ligue; se continuar, anote o código.", False))
    return Problem("Epson", "0x%02X" % code, title, cause, fix, service)


def explain_canon(code):
    code = code.upper()
    if code not in CANON_CODES:
        return Problem("Canon", code, "Código %s não catalogado" % code,
                       "Este código não está na lista do manual da G2060.",
                       "Consulte o manual do seu modelo em ij.manual.canon.", False)
    title, cause, fix, service = CANON_CODES[code]
    return Problem("Canon", code, title, cause, fix, service)


def search(text):
    """Busca por código ou palavra nas duas bases."""
    text = text.strip().upper()
    found = []
    for code in CANON_CODES:
        p = explain_canon(code)
        if text == code or (len(text) > 2 and text in (p.title + p.cause).upper()):
            found.append(p)
    for code in EPSON_ERRORS:
        p = explain_epson_error(code)
        if text in (p.code, "%X" % code) or (len(text) > 2 and text in (p.title + p.cause).upper()):
            found.append(p)
    return found
