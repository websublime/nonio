# nonio-scan

The TS/JS extraction backend: `OxcExtractor` implements `nonio_facts::Extractor` with one oxc 0.153 parse and one
walk per file. It is a port of illuminate's `packages/facts` (extract, record, resolver, entries, adapters, tables,
routes, regex fallback) and gives the same facts for the same files, with the differences listed below.

```rust
let ex = nonio_scan::OxcExtractor::new(root);
let facts = ex.extract("src/app.ts", &source); // Some(FileFacts); None for files it does not handle
```

`.java`, `.kt`, `.kts` and `.py` files return `None` here: they belong to `GraphExtractor` (below). `.go` files get
no facts yet. `Scanner` sends each file to its backend:

```rust
let scanner = nonio_scan::Scanner::new(root, files); // files: the repo's files, e.g. from list_repo_files
```

## Java, Kotlin and Python (D14)

`GraphExtractor` reads these files with `code2graph` 0.0.0-beta.17 (Apache-2.0), tier `default_scoped`, and resolves
them together, so a call crosses files; the result is kept until one of the texts changes. Symbol ids follow the
contract: `<path>#<Type.member>` (`OrderService.java#OrderService.create`, `main.py#get_user`, `Taxes.kt#formatAmount`);
Java overloads are one symbol, as `code2graph` keeps the first. Imports are repo files, or `vendor:<package>`.

The precise tier still makes three kinds of edge that rest on a name, measured in `tests/graph.rs`, and the adapter
drops them (D16):

| `code2graph` rule | Wrong edge it made | Kept only when |
|---|---|---|
| A qualified call is a path lookup | `def notify(client): client.send()` → `app/client.py#send`; Kotlin `billing.charge()` on a constructor property → package `billing` | the receiver is an import, a type, or a name the file doesn't bind |
| An unbound name falls back to the package | Java `this.store.persist()` (read without its receiver) → the one `persist` in the package | the callee is a class, a top-level function, a member of an enclosing type (or one it extends), or a static import |
| Types are looked up by bare name | `m: Mailer` (imported from `b`) → `c.Mailer.dispatch`, when `c` is read first | the type name resolves where it is written (same file, explicit import, same package) to the member's owner |

### Typed receivers and constructors (D15)

What `code2graph` leaves unresolved, or resolved by name and dropped above, is resolved again from the types written
in the code: the receiver is a field or a Kotlin constructor property of an enclosing type (`orderService.create()`,
`this.repo.save()`), a local or parameter with a declared type, or a local bound once to a constructor call
(`repo = UserRepo()` in Python); and `new Order(…)` in Java (the class and its constructor share the name, so
`code2graph` links neither). The type name is read where it is written (same file, explicit import, same package,
then packages imported with `.*` when exactly one has the name) and the method looked up on that type, then on the
nearest types it extends; any doubt gives no call. These calls are `typed`.

On the phase 2 fixture (`tests/fixtures/spike05`, the spike of `05-spike-crates.md` rebuilt with its decoys) the
scanner finds 20 of the 20 calls, none wrong. On five public repos, 216 calls checked by hand, none wrong
([Q10](../../docs/spikes/q10-code2graph.md)).

Known gaps (recall only): chained calls (`a().b()`), receivers inside `apply {}`/`with {}`, inherited fields,
`self.x` in Python, Java overloads after the first, names under 3 characters.

Routes and entries for these languages come later (phase 4).

## Vue single-file components (D28)

A `.vue` file is read as one oxc program, with every position an offset in the `.vue`:

- The `<script>` and `<script setup>` blocks are parsed in place: the text outside them is blanked, so the bindings of
  `<script setup>` are top-level symbols (`OrderList.vue#refresh`) and their calls are found as in a TS file.
- The template is read with `vue-compiler-core` and lowered to JSX in the same program (ported from `vue_oxlint_jsx`,
  MIT, see NOTICE), so its names resolve by scope against the script: `v-for` variables and slot props are
  parameters of arrows around their element, and `v-if` sits outside the `v-for`. Its calls belong to the symbol
  `<path>#<template>`, whose span is the `<template>` block. A handler given by reference (`@click="refresh"`) counts
  as a call, as Vue calls it.
- Without `<script setup>` (Options API) the template sees the component instance, not the module, so it gives no
  calls rather than calls linked by name (D16). A template in another language (`lang="pug"`), an expression that
  doesn't parse (that expression only) and a `v-for`/`v-slot` whose variables don't parse (that element) are left
  out. Markup that doesn't parse (an unclosed tag) falls back to imports only, as any file that doesn't parse.

Tested in `tests/vue.rs`, which ports the Q11 spike harness and its 16 expectations
(`docs/spikes/q11-vue.md`).

## How it is checked

- The `nonio-core` tests run the checker on facts from this crate and assert, file by file, that they equal the facts
  recorded from illuminate's own extractor (`crates/nonio-core/tests/common/mod.rs`, `Oxc`).
- `tests/` ports illuminate's facts tests (`extract`, `imports`, `resolve`, `tables`).
- [`tools/compare-facts`](../../tools/compare-facts/README.md) diffs both extractors over any set of files.

## Where it differs from illuminate

- **Parameters and local variables never link by name (D16).** illuminate links a call whose root is a parameter or a
  local variable to an import or a top-level symbol of the same name (`function f(url) { url.parse() }` next to
  `import url from 'node:url'` gives a call to `vendor:node:url`). nonio resolves the root with oxc_semantic: a
  binding below the module scope links only to the nested function or captured `import()`/`require()` it actually
  is, and otherwise gives no call. Over about 8,000 files of `node_modules` this drops 491 of 58,952 call edges
  (0.8%); every one sampled was a false name match.
- **Large and minified files go through oxc too.** For files over 128,000 characters, and minified ones, illuminate
  reads imports with es-module-lexer, which takes `require()` calls inside comments as imports, marks some type-only
  imports differently, and reads `export { a } from` re-exports as local names. nonio builds every record from oxc's
  module record (the walk itself still skips files over 500,000 UTF-16 units and minified files, as in the
  original), so imports are the ones the code has and calls follow re-exports to where the symbol is defined.
- **Uninstalled workspace packages resolve without writing.** illuminate links workspace packages into a folder on
  disk so its resolver can find them. nonio gives `oxc_resolver` a file system that shows those links virtually under
  `<root>/.nonio/ws-modules`; nothing is written to the repo (D25).
