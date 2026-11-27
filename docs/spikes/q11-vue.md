# Spike Q11 — Vue SFC com `vue-oxlint-toolkit`

Corrido em 2026-11-27. Reproduzível com [`tools/spike-vue`](../../tools/spike-vue/README.md) (fixture, harness,
expectativas, output bruto em `results-*.txt`).

## Conclusão

**O `vue-oxlint-toolkit` faz o que precisamos, mas não pode ser dependência do `nonio`.**

- **Funcionalmente serve.** O `vue_oxlint_jsx` 0.15.0 junta script, `<script setup>` e template num só `Program` do
  oxc. Com `oxc_semantic` por cima, acertou as 16 expectativas do fixture: chamadas no topo e dentro de funções do
  `<script setup>`, referências do template a funções, composables e componentes, a variável do `v-for` a não ser
  confundida com um homónimo, e todas as posições a bater certo com o `.vue` original.
- **Não compila com o oxc do `nonio`.** Declara `oxc_* >= 0.129`, mas só compila com oxc 0.129 a 0.135 (medido);
  com 0.136 dá 3 erros, com 0.140 dá 7 e com 0.153 dá 31. O `vue_oxlint_parser` também não compila com 0.153 (28
  erros). O `nonio` está fixo em oxc 0.153 (MSRV 1.97).
- **O projeto está parado e diz-se não utilizável.** O README diz "early proof of concept. It can't be used directly
  for now". Último release a 2026-05-24; desde então um só commit de documentação (2026-10-03). Os testes do
  `vue_oxlint_jsx` estão desligados desde 2026-05-16 ("TODO: enable tests after migration").
- **Sem crate nenhum, o `nonio-scan` já cobre o script.** Com o texto fora dos blocos `<script>` trocado por espaços
  (posições preservadas), o backend oxc atual dá as chamadas do `<script setup>` com os ids e spans do `.vue`. Falta o
  template.

**Decisão (D28, Acordado em 2026-11-27):** backend Vue próprio no `nonio-scan`, sobre o oxc do workspace. Script
lido no sítio; template baixado para JSX com o método do `vue_oxlint_jsx` (MIT), portado para oxc 0.153, e
`vue-compiler-core` 0.1.0 (MIT, sem dependência de oxc) para o HTML, como faz o próprio `vue_oxlint_jsx`.

## Pergunta

O `vue-oxlint-toolkit` cobre o `<script setup>` e o template como o checker precisa? Três coisas:

1. Chamadas no topo e dentro de funções do `<script setup>`.
2. Referências no template a funções e composables.
3. Posições que mapeiam para o ficheiro `.vue` original.

## Método

- **Fixture:** o `spike-facts-crates.tar.gz` do 05 não está disponível, por isso foi recriado um equivalente:
  `OrderList.vue` (`<script setup lang="ts">` com composable, cliente de API, `computed`, `onMounted`, template com
  interpolação, evento, `v-for` e componente filho), `OrderRow.vue` (`defineEmits` usado no template),
  `LegacyCounter.vue` (Options API, `<script>` simples), mais `useOrders.ts`, `api/client.ts` e `utils/format.ts`.
- **Iscos:** `o.load()` no template, onde `o` é a variável do `v-for`, com dois homónimos `load` noutros sítios
  (`useOrders().load` e `utils/format.ts#load`); `show()` no template de Options API, que não é binding de scope.
- **Expectativas:** 16, escritas à mão em `fixture/expected.txt`. Cada uma exige o texto como está no `.vue`, a linha
  certa, `source[span] == texto` e o alvo certo.
- **Crates:** `vue_oxlint_jsx` 0.15.0 e `vue_oxlint_parser` 0.15.0 (MIT), com oxc 0.129.0 (a versão do `Cargo.lock`
  do upstream). Base de comparação: `nonio-scan` do `main` (8b70279), oxc 0.153.0.
- **O que o harness mede:** a resolução de nomes do `oxc_semantic` sobre o `Program` do crate (se a raiz de cada
  chamada é um import, um binding do `<script setup>`, um binding do template, ou nada). É o pré-requisito para o
  `Walker` do `nonio-scan`; não são ainda os `calls` do contrato de factos.

## Resultado

### `vue_oxlint_jsx` 0.15.0 + `oxc_semantic` 0.129: 16 de 16

| Caso | Resultado |
|---|---|
| `<script setup>`, chamada no topo (`useOrders()`, `computed()`, `onMounted()`) | Resolvida ao import |
| `<script setup>`, chamada dentro de função (`http.get` em `refresh`) | Resolvida ao import |
| `<script setup>`, chamada a função local (`refresh()` dentro de `onMounted`) | Resolvida ao binding do setup |
| Macro do compilador (`defineProps`, `defineEmits`) | Não resolvida (correto: não é import nem binding) |
| Template, chamada a import (`{{ formatTotal(total) }}`) | Resolvida ao import |
| Template, handler por referência (`@click="refresh"`) | Resolvida ao binding do setup |
| Template, chamada a função do setup (`@select="pick(o.id)"`, `emit('select')`) | Resolvida ao binding do setup |
| Template, componente (`<OrderRow>`) | Resolvida ao import de `./OrderRow.vue` |
| Isco: `o.load()` com `o` do `v-for` | Resolvida a `o` do template; nenhum homónimo |
| Options API, `<script>` simples (`load()`, `formatTotal()`) | Resolvidas ao binding e ao import |
| Options API, método no template (`{{ show() }}`) | Não resolvida (correto) |

Posições: as 31 chamadas e referências vistas têm `source[span]` igual ao texto no `.vue` (zero desvios); as
declarações também (`refresh` em 12:10, `emit` em 3:7). Nenhum erro de parse nem de semântica.

Duas particularidades medidas que contam para a integração:

- **O `<script setup>` fica dentro de uma arrow function sintética.** O `Program` tem os imports no topo e depois
  `async () => { …setup…; <>…template…</> }` com span `0..0`. Os bindings do setup ficam a profundidade 1, não no
  scope do módulo.
- **Há nós sintéticos com span vazio.** O `v-for` gera uma chamada em `0..0`. Têm de ser filtrados antes de chegarem
  aos factos.

### Compatibilidade com o oxc do `nonio`

| oxc | `vue_oxlint_jsx` 0.15.0 |
|---|---|
| 0.129, 0.130, 0.131, 0.133, 0.134, 0.135 | Compila |
| 0.136 | 3 erros |
| 0.140 | 7 erros |
| 0.153 | 31 erros (`AstBuilder` e `NONE` mudaram de sítio, campos de AST renomeados, `ParserReturn` mudou) |

Duas versões de oxc no mesmo binário compilam, mas o AST do crate (0.129) não é o tipo que o `Walker` do
`nonio-scan` (0.153) lê. Usar o crate obrigaria a fixar o workspace inteiro em oxc ≤ 0.135, o que desfaz a subida
para 0.153.

Custo de descer o `nonio` para 0.135 (o teto do crate), medido a compilar o `nonio-scan` do `main`: 13 erros de
compilação com 0.135 e os mesmos 13 com 0.129, todos mudanças de API (`ArrowFunctionBody`, `Class.heritage`,
`ParserReturn.diagnostics` e `fatal_error`, variantes de export). O oxc 0.135 pede Rust 1.94. Não medido: se os
factos continuam iguais aos gravados do `illuminate` (oxc 0.153) depois do porte; e cada subida de oxc ficaria
presa ao crate.

### Base: `nonio-scan` (oxc 0.153) sem crate Vue

| Modo | `OrderList.vue` |
|---|---|
| Hoje (o `.vue` passado como está) | Não faz parse como TS; o fallback por regex dá os 5 imports e nenhum símbolo nem chamada |
| Fora dos `<script>` trocado por espaços | 4 símbolos (`refresh`, `pick`, 2 callbacks) e as chamadas `useOrders`, `vue`, `refresh → http.get`, `<cb2> → refresh`, com spans do `.vue` |

O `load()` dentro de `refresh` fica sem aresta nos dois casos, como em TS: `load` vem de desestruturar o retorno de
`useOrders()` e só se ligaria por nome (D16).

## Medido e assumido

| Afirmação | Estado |
|---|---|
| `vue_oxlint_jsx` resolve script setup, template e posições no fixture | Medido, 16/16 |
| Compila só com oxc 0.129 a 0.135 | Medido (0.129 a 0.131, 0.133 a 0.136, 0.140, 0.153) |
| O projeto está parado e não é para uso direto | Medido (README e `git log` do upstream em 2026-11-27) |
| O `nonio-scan` atual cobre o script com o texto fora dos `<script>` trocado por espaços | Medido no fixture |
| Portar a baixa do template para oxc 0.153 é trabalho contido (cerca de 31 erros de compilação, sobretudo mudanças de API) | Medido: feito (ver "Implementação") |
| `vue-compiler-core` 0.1.0 (2021) aguenta SFC reais | Assumido; só corrido no fixture, através do `vue_oxlint_jsx` |
| Resultado igual num repo Vue real | Não medido; o fixture é pequeno e escrito por quem fez o spike |
| `vue_oxlint_parser` (porte do `vue-eslint-parser`) | Só compilado com 0.129; não medido |

## O que o `nonio-scan` precisa para integrar Vue

1. **Detetar `.vue`** no `OxcExtractor` (hoje vai para o parse como TS e cai no fallback).
2. **Script no sítio:** passar ao parse do oxc 0.153 o texto do `.vue` com tudo fora dos blocos `<script>` trocado por
   espaços. As posições ficam as do `.vue` sem tabela de mapeamento. `lang` do bloco decide `SourceType`.
3. **Template:** baixar o template para expressões JSX no mesmo `Program`, para o `oxc_semantic` resolver os nomes
   contra o scope do setup, com `v-for` e `v-slot` a abrir scopes próprios (é o que o `vue_oxlint_jsx` faz em
   `parser/elements/{v_for,v_slot}.rs`). Ao contrário do crate, pôr o setup no scope do módulo e não numa arrow, para
   o `Walker` ver `refresh` como símbolo de topo (`OrderList.vue#refresh`) e não como local (D16).
4. **Filtrar nós sintéticos** (span vazio) antes de emitir factos.
5. **De quem é uma chamada do template.** Implementado com a proposta (símbolo `<path>#<template>`), à espera de
   confirmação do Miguel. Era por decidir: do ficheiro (como as chamadas de topo hoje) ou de um símbolo
   sintético `<template>`. Proposta: símbolo `<path>#<template>` com o span do bloco, para o mapa poder mostrar
   "o template chama `refresh`".
6. **NOTICE:** o que for portado do `vue_oxlint_jsx` leva o aviso MIT de Liang Mi (e o que ele traz de
   `vue-oxc-parser`, de zhiyuanzmj).

## Propostas de alteração aos documentos

### 05-spike-crates.md

Na tabela "Outros crates vistos, sem spike", tirar `vue_oxc_toolkit` e acrescentar uma secção:

> ### Vue: `vue-oxlint-toolkit` (spike Q11, 2026-11-27)
>
> `vue_oxlint_jsx` 0.15.0 (MIT) junta script, `<script setup>` e template num `Program` do oxc com posições do
> `.vue`. Com `oxc_semantic`: 16 de 16 num fixture Vue próprio (chamadas no topo e dentro de funções do setup,
> referências do template, isco do `v-for`, posições exatas). Só compila com oxc 0.129 a 0.135 (31 erros com 0.153);
> README diz "proof of concept", sem releases desde 2026-05-24. Não serve como dependência. Detalhe em
> `docs/spikes/q11-vue.md` do repo.

### 06-roadmap-e-questoes.md

- Q11 passa a fechada: "Medido em 2026-11-27: cobre script setup e template, mas não compila com oxc 0.153 e está
  parado. Ver D28."
- Fase 2, objetivo: "`scan`: contrato, backend oxc (TS/JS), backend Vue próprio (D28), adapter `code2graph`, resolver
  de campos".
- Riscos, nova linha: "`vue-compiler-core` parado desde 2021 | SFC com sintaxe recente partem o template | Testes com
  SFC reais; o script não depende dele".
- Fontes: `https://github.com/liangmiQwQ/vue-oxlint-toolkit`, `https://crates.io/crates/vue_oxlint_jsx`,
  `https://crates.io/crates/vue-compiler-core`.

### 03-arquitetura.md, §Backends

Linha Vue SFC: "oxc sobre os blocos `<script>` lidos no sítio, mais o template baixado para JSX no mesmo `Program`
(D28)".

### 02-decisoes.md, Scanner

| # | Decisão | Porquê | Estado |
|---|---|---|---|
| D28 | Backend Vue próprio no `nonio-scan`, sobre o oxc do workspace: script lido no sítio, template baixado para JSX com o método do `vue_oxlint_jsx` (portado, MIT, no NOTICE) e `vue-compiler-core` para o HTML; sem depender do `vue-oxlint-toolkit` | No spike Q11 o crate acertou 16/16, mas só compila com oxc ≤ 0.135, está parado e diz-se proof of concept; um segundo oxc no workspace não serve ao `Walker` | Acordado |

## Implementação (2026-11-27)

Feita no `nonio-scan` (`src/vue.rs`), sobre oxc 0.153 e `vue-compiler-core` 0.1.0, sem o `vue-oxlint-toolkit`.
Detalhe no [`README` do crate](../../crates/nonio-scan/README.md#vue-single-file-components-d28).

| Afirmação | Estado |
|---|---|
| As 16 expectativas do fixture passam com o porte | Medido: `tests/vue.rs` corre o harness do spike portado sobre `tests/fixtures/vue-app`; 16/16 |
| Posições: todas as chamadas, referências e nomes do template fazem `source[span] == texto` no `.vue` | Medido no fixture e num `.vue` com texto não ASCII antes dos blocos |
| `refresh` e `pick` do `<script setup>` são símbolos de topo (`OrderList.vue#refresh`), sem arrow sintética | Medido |
| Nenhum nó sintético chega aos factos | Medido: o `v-for` passa a `[fonte, (vars) => <el/>]` (sem chamada); a única chamada sintética é a do handler por referência, cujo callee tem o span real |
| `vue-compiler-core` 0.1.0 aguenta SFC reais | Assumido; só corrido no fixture e nos casos de `tests/vue.rs`. Não medido num repo Vue real |
| Custo do `vue-compiler-core` | Medido: 42 crates novos no `Cargo.lock` (sobretudo `rslint_parser` e os seus, e `winapi`); compila com Rust 1.97 |

Diferenças medidas em relação ao `vue_oxlint_jsx`, e porquê:

- **Posições em chars.** O `vue-compiler-core` conta `Position.offset` em chars, não em bytes
  (`scanner.rs`, `move_by`). O `vue_oxlint_jsx` usa-os como bytes; num `.vue` com texto não ASCII antes de um bloco
  os spans ficariam desviados. O `nonio` converte-os para bytes.
- **Sem `<script setup>` (Options API), o template não é baixado.** Nesse caso o template vê a instância do
  componente e não os bindings do módulo; ligar `{{ show() }}` ou `{{ formatTotal() }}` ao que o script declara seria
  ligar por nome (regra 7, D16). A expectativa do `show` passou de "não resolvido" a "não baixado".
- **Handler por referência conta como chamada.** `@click="refresh"` dá `OrderList.vue#<template> → #refresh`, porque o
  Vue chama o handler no evento. Handlers com várias instruções (`a = 1; close()`) passam a corpo de uma arrow.
- **Scope que não faz parse larga o elemento.** Um `v-for` ou `v-slot` cujas variáveis não fazem parse não deixa os
  nomes do elemento sem scope (que se ligariam por nome); o elemento sai inteiro.
- **Cada expressão faz parse sozinha**, com os spans deslocados para o sítio no `.vue`, em vez de copiar o ficheiro
  inteiro com padding por expressão. Linear no tamanho do template e sem `unsafe`.
- **Componentes usados no template não são chamadas**, como no TSX (a aresta de import já existe). O nome do componente
  resolve-se ao import, mas não entra em `calls`.
