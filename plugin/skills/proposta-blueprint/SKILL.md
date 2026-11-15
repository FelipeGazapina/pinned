---
name: proposta-blueprint
description: Como o Felipe quer receber uma proposta de arquitetura ou de produto. Uma página de proposta com o desenho da arquitetura numa folha azul de engenheiro (letras brancas à mão), e depois um protótipo clicável das telas para ele validar. Use quando ele pedir "design", "desenho da arquitetura", "proposta", "como seria", "folha azul", "blueprint" ou "protótipo" de algo novo no pinned ou em outro projeto.
---

# Proposta com folha azul e protótipo

São duas entregas, nesta ordem. A segunda só começa quando o Felipe pedir ou aprovar a primeira.

1. **Página de proposta**: um Artifact HTML com o texto da proposta e o desenho da arquitetura em folha azul.
2. **Protótipo**: um Artifact do tipo Design (canvas com uma tela por artboard), clicável, com a cara do produto.

Nenhuma das duas é código do projeto. Código só começa quando o Felipe mandar.

## Antes de escrever

- Leia o que existe. Use `repo_overview` e poucas chamadas de `repo_search` ou `repo_read` para saber o que já está pronto e o que seria novo. O desenho separa as duas coisas.
- Procure na memória (`memory_search`) decisões que o Felipe já tomou sobre o assunto. Uma decisão dele entra na página como decidida, não como pergunta.
- Guarde a direção nova na memória (`memory_save`) quando ele refinar a ideia.

## Entrega 1: a página de proposta

Carregue a skill `artifact-design` e publique um arquivo `.html` do scratchpad com a ferramenta Artifact. Parta de `template.html`, que está nesta pasta: ele já traz as cores, as fontes, a folha azul e o carimbo.

Ordem das seções:

1. Título e um parágrafo que diz a ideia inteira em linguagem simples.
2. As duas ou três regras que sustentam o desenho.
3. A folha azul (figura larga, com legenda embaixo).
4. O que muda e o que continua igual (tabela).
5. Como funciona, passo a passo, um fluxo por lista numerada.
6. Papéis ou permissões, se houver (tabela sim/não).
7. Escolhas de desenho que recomendo: um cartão por escolha, com o motivo e a alternativa descartada.
8. O que precisa existir antes.
9. Fases, cada uma com a prova de que funciona.
10. Decisões que são dele: lista numerada, cada item com a minha recomendação.
11. Nota final: o que eu li de verdade, o que não li, e qual é o próximo passo.

Regras do texto:

- Português, frases curtas, sem jargão. Diga primeiro o que importa.
- Cada recomendação traz o motivo. Cada decisão em aberto traz a opção que eu escolheria.
- Dados na tela são inventados. Nunca nomes de clientes, e-mails, chaves ou senhas.
- Diga com clareza o que foi verificado no código e o que é suposição.

### A folha azul

O desenho é um `<svg>` escrito à mão, dentro de `<div class="sheet">` em uma `<figure>`. Carregue a skill `artifact-diagramming` antes de desenhar.

- **Papel**: azul `#1b4a93`, com grade fina a cada 20 px e grade forte a cada 100 px. A cor é fixa: a folha fica azul no tema claro e no escuro.
- **Tinta**: branca `#f3f7ff`. Texto secundário em `#b9cdee`.
- **Letra**: "Architects Daughter", 12 a 15 px. Títulos de coluna em maiúsculas.
- **Tamanho**: `viewBox="0 0 1200 800"`, com `min-width: 980px` e rolagem lateral no celular.
- **Moldura**: um retângulo a 12 px da borda.
- **Carimbo** no canto inferior direito: nome do projeto, "ARQUITETURA PROPOSTA", "FOLHA 1/1 · REV A", data e "S/ ESCALA". A cada revisão a letra sobe (REV B, REV C).
- **Traço contínuo** (classe `bp-new`) é o que é novo. **Traço tracejado** (classe `bp-old`) é o que já existe hoje. A legenda no canto inferior esquerdo diz isso.
- **Colunas**: quem usa à esquerda, o servidor no meio, onde o trabalho roda à direita. Cada coluna tem um título no topo.
- **Setas** com rótulo de uma a três palavras, em duas linhas (uma acima e uma abaixo do fio). A seta mais importante da proposta é mais grossa.
- Alinhe tudo na grade de 20 px. Nenhum texto encosta em uma linha.
- A `<svg>` tem `role="img"` e um `aria-label` que conta o desenho em uma frase. A `<figcaption>` diz o mesmo para quem lê.

Eu não vejo a página renderizada. Diga isso ao Felipe e peça que avise se algo ficou encavalado.

### Revisão

Quando ele refinar a ideia, edite o mesmo arquivo e publique de novo no mesmo link. Suba a letra da revisão no carimbo. Na resposta, liste o que mudou e as escolhas que eu fiz e que ele pode trocar.

## Entrega 2: o protótipo

Para o Felipe, "design" e "protótipo" são o Artifact do tipo Design. A primeira chamada é `Artifact` com `action: "quickstart"` e `intent: "design"`. Siga as instruções que o tipo devolve.

- Uma tela por artboard, ligadas por links, para ele ver todas lado a lado e também clicar no modo Play.
- Use a cara do produto. Para o pinned, as cores estão em `ui/gui-theme.css`: fundo `#0a0a0a`, barra lateral `#000000`, cartão `#111111`, texto `#f5f5f5`, texto secundário `#a3a3a3`, botão principal `#346bf1`, sucesso `#00bc7d`, aviso `#fe9a00`, borda branca a 8%.
- Carregue a skill `frontend-design` antes de desenhar.
- Desenhe pelo dia a dia de quem usa, não pelo menu nem pela arquitetura. A primeira versão do protótipo do Teams foi recusada pelo Felipe em 2026-11-15 porque "parecia um monte de tela de configuração": listas, tabelas e painéis de estado.
- A tela inicial é a coisa principal que a pessoa faz ali (no pinned, conversar e pedir trabalho). Pessoas aparecem com rosto e frase ("Carla está atualizando a página de pedidos"), não como linhas de tabela com etiqueta de tipo.
- Uma ação acontece onde a pessoa está: o botão de aprovar aparece dentro da conversa ou no início, não numa tela de fila.
- Configuração, papéis, orçamento e conectores ficam fora do protótipo, a não ser que ele peça. Use palavras de quem usa ("publicação de teste"), não as do sistema ("run", "schedule", "box").
- Cada tela prova uma decisão da proposta.
- Quando uma tela só faz sentido para um papel (Admin aprova, Owner configura), mostre no rodapé da barra lateral de quem é a visão.
- Um ou dois controles funcionam de verdade em cada fluxo que importa (aprovar, pausar, promover). O resto é estático.
- Dados inventados e plausíveis. Sem estatísticas soltas e sem texto de enchimento.
- Escreva um arquivo, publique, e só então escreva o próximo.

Na resposta, diga o que cada tela mostra, quais botões funcionam e o que foi suposição minha. Quando ele aprovar, o protótipo vira a referência: a tela construída depois é comparada com ele por screenshot.

## O que entregar na conversa

Uma resposta curta em português com o link, o que cada parte mostra, as escolhas que eu fiz e as decisões que dependem dele. Os dois Artifacts são privados até ele compartilhar.
