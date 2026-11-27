# Spike Q10 — precisão do `code2graph` em repos reais

Corrido em 2026-11-27 com `code2graph` 0.0.0-beta.17, tier `default_scoped`, atrás do adapter e do resolver de
recetores tipados do `nonio-scan` (`src/graph.rs`). Reproduzível com o exemplo
[`graph_calls`](../../crates/nonio-scan/examples/graph_calls.rs):

```sh
cargo run --release -p nonio-scan --example graph_calls -- <repo> summary|kept|dropped
```

## Conclusão

**O `code2graph` serve como dependência, mas só atrás do adapter: o tier preciso, sozinho, não falha fechado.**

- **Com o adapter: 0 erradas em 216 chamadas verificadas à mão**, em cinco repos públicos (Java, Kotlin, Python e um
  Spring + Vue). Com zero erros em 216, o limite superior a 95% para a taxa de erro é cerca de 1,4%.
- **Sem o adapter, o tier preciso faz arestas erradas em código real**, ao contrário do que o fixture do `05` sugeria
  (10 certas, 0 erradas). No spring-petclinic, `LocalDate.now().toString()` liga a `Owner.toString` (4 vezes): o
  nome sem ligação cai no único `toString` do pacote. No mall, `getCacheService().delAdmin()` liga à classe
  `UmsAdminCacheServiceImpl` em vez da interface que o método devolve (11 vezes). No fixture recriado apareceram mais
  três formas (ver `crates/nonio-scan/README.md`). O adapter descarta todas.
- **O que o `code2graph` deixa por resolver, o resolver próprio (D15) cobre** sem perder precisão: campos,
  propriedades de construtor Kotlin, locais e parâmetros com tipo escrito, e `new X()` em Java. É a maior parte das
  chamadas mantidas nos repos Java (ver a tabela).
- **Desempenho aceitável, não ótimo.** 524 ficheiros Java em 5,2 s (release), refeitos sempre que um ficheiro muda.
  Medir em repos maiores antes de servir o checker em modo interativo.

O `05` pedia este passo antes de fixar o `code2graph`; com o resultado acima, D14 mantém-se, com uma nota: o adapter
volta a verificar cada aresta (D16).

## Método

- **Repos** (clone superficial, commit indicado): `spring-projects/spring-petclinic` (3aae6cc, Java),
  `spring-petclinic/spring-petclinic-kotlin` (9823b3a), `fastapi/full-stack-fastapi-template` (f8c6c14, Python),
  `jonashackt/spring-boot-vuejs` (86d9f5b, Spring + Vue), `macrozheng/mall` (dcaa93b, Java, 524 ficheiros, muitos
  nomes repetidos entre módulos).
- **Amostra:** chamadas mantidas escolhidas ao acaso (`random.seed` 10 a 15): 40 do petclinic, 30 do Kotlin, 30 do
  FastAPI, 60 + 40 do mall (as 40 depois de acrescentar os imports `.*`), e as 16 do spring-boot-vuejs. Cada uma
  conferida contra o código: o tipo do recetor onde está declarado, o import, e o método nesse tipo ou num supertipo.
- **Arestas descartadas:** as do tier preciso que o adapter não manteve, todas lidas.
- O lado Vue do spring-boot-vuejs vai pelo backend oxc e não entra aqui.

## Resultado

| Repo | Ficheiros | Chamadas mantidas | Syntactic / Typed / Dispatch | Verificadas | Erradas | Descartadas do tier preciso |
|---|---|---|---|---|---|---|
| spring-petclinic | 50 | 342 | 81 / 257 / 4 | 40 | 0 | 5: 4 erradas, 1 certa por acaso |
| spring-petclinic-kotlin | 40 | 102 | 33 / 69 / 0 | 30 | 0 | 1: certa por acaso (`addSpecialty` dentro de `apply {}`) |
| full-stack-fastapi-template | 43 | 234 | 234 / 0 / 0 | 30 | 0 | 0 |
| spring-boot-vuejs | 10 | 16 | 5 / 11 / 0 | 16 | 0 | 0 |
| mall | 524 | 2292 | 795 / 1497 / 0 | 100 | 0 | 11: alvo pelo nome (a classe `Impl`, não a interface) |

"Certa por acaso": o alvo estava certo, mas o `code2graph` chegou lá só pelo nome; o adapter não a pode distinguir das
erradas e perde-a.

## Encontrado e corrigido durante a medição

- **Imports de classes com construtor** (petclinic): em Java e Kotlin a classe e o construtor têm o mesmo nome, e o
  `code2graph` não liga um import a um nome ambíguo. O import saía como `vendor:` e os recetores desse tipo perdiam-se.
  O adapter resolve-o pelo caminho completo.
- **Imports `.*`** (mall): `import com.macro.mall.model.*` não gera referência no `code2graph`. Sem eles o adapter
  perdia cerca de 300 chamadas certas. Agora segue a ordem do Java: import explícito, pacote, pacotes `.*` (só se um
  único tiver o nome).

## Limites

- Repos públicos, pequenos a médios; nenhum com a mistura Spring + Vue de uma aplicação interna a sério.
- Recall não medido: o `code2graph` não diz quantas chamadas de cada sítio são para código do repo.
- Lacunas conhecidas, todas de recall: chamadas encadeadas (`a().b()`), recetores dentro de `apply {}`/`with {}`,
  overloads (o `code2graph` guarda só o primeiro; as chamadas de dentro dos outros ficam atribuídas à classe), campos
  herdados, `self.x` em Python, nomes com menos de 3 caracteres (o extrator ignora-os).
- Uma versão do `code2graph`, em beta; repetir este spike quando subir de versão.
