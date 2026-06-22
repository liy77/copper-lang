# Alloy — VM de iteração para Copper

**Data:** 2026-06-22
**Status:** Design aprovado (pré-implementação)

## Resumo

Alloy é uma **VM de bytecode** para Copper, distribuída como um binário separado
(`alloy`), pensada para iteração rápida: `alloy run`, `alloy repl`, hot-reload
de MUI e `alloy build` (binário self-contained). Ela **não substitui** o caminho
de produção atual (transpile `.crs` → Rust → `cargo build`, dono do `cforge`),
que continua sendo o alvo de release nativo.

Decisão central: **não forkar o `rustc`.** O caminho nativo já existe via
transpiler e é 100% compatível com Rust por construção. Alloy adiciona um alvo
*interpretado* sobre a mesma AST, sem tocar no backend nativo.

### Invariante de compatibilidade

O mesmo `.crs` deve produzir o mesmo comportamento observável nos dois alvos
(`alloy run` e `cforge run`). Isso é verificado no CI (harness de paridade) e é
a forma concreta de "Alloy continua compatível com Rust".

## Arquitetura

```
                 crates/copper-syntax::program  (Program AST — JÁ EXISTE)
                                   │
                  ┌────────────────┴────────────────┐
                  ▼                                  ▼
        crates/alloy-vm                       crates/copper-parser
        (interpretador tree-walking)          (AST/tokens → Rust source)  ← já existe
                  │                                  │
                  ▼                                  ▼
        bin: alloy                               cargo build
        (run / build / repl / check)           (binário nativo, release)
```

**Princípios**

- **AST única e compartilhada** (`copper-syntax`) é a fonte de verdade dos dois
  alvos. Nenhum backend re-tokeniza.
- **`alloy` é binário separado** do `cforge`. `cforge` continua dono do
  transpile/release; `alloy` é dono da VM.
- **Crates novos isolados** (`alloy-compiler`, `alloy-vm`). `cforge` e `mui-*`
  não dependem deles, então o CI Linux/macOS/Windows não quebra por causa do
  Alloy.

## Componentes

### 1. `copper-syntax` (JÁ EXISTE — fundação pronta)

**Atualização (2026-06-22):** a AST completa **já está implementada** no repo,
não precisa ser construída:

- `crates/copper-syntax/src/expr.rs` — AST tipada completa (`Expr`/`ExprKind`,
  `Stmt`, `Pattern`, `Block`, `Type`) + Pratt parser (`parse_expr`,
  `parse_stmts`, `parse_stmts_tokens`). Cobre literais, member, call (turbofish),
  path, index, cast, try, unary/binary, ternário, assign, range, array, closure,
  struct-lit, `if`/`match` como expressão, block. Fallback `ExprKind::Raw` para
  o que o subset ainda não forma.
- `crates/copper-syntax/src/program.rs` — `parse_program(source) -> Program`,
  AST de programa inteiro: `Item` (func/struct/class/impl/trait/enum/import/stmt)
  com **corpos `Block` totalmente parseados** (reusa o Pratt parser de `expr`).
  Descrita no próprio arquivo como "the faithful program AST a backend
  (Cranelift, or a re-emitter) can walk".
- `crates/copper-syntax/src/ast.rs` — árvore span-based nível-item (corpos =
  span), usada pelo LSP; permanece como está.

Logo o primeiro trabalho de Alloy **não** é AST, é o backend. Gaps residuais
(casos que ainda caem em `Raw`, genéricos em itens) são tratados sob demanda
conforme a VM os exercita, não como uma fase prévia.

### 2. `alloy-vm` (runtime — interpretador tree-walking primeiro)

Decisão de implementação: começar como **interpretador tree-walking** que
executa o `Program` de `program.rs` diretamente — caminho mais curto até
`alloy run` funcional, reaproveita 100% da AST e é trivial de depurar.
Bytecode de pilha e/ou JIT Cranelift são **otimizações posteriores** (a AST já
suporta Cranelift), cada uma em seu próprio plano.

- Loop de avaliação por nó da AST: ambiente de escopos encadeados + frames de
  chamada.
- Modelo de valores: `Int`/`Float`/`Bool`/`Str`/`Vec`/`Map`/`Struct`/`Enum`/
  `Closure`/`Unit`.
- **Memória:** ownership/move/borrow **simulados em runtime** (não estáticos).
  Move invalida a origem; borrows checados dinamicamente. Mantém a semântica
  Rust *observável* sem reimplementar o borrow-checker.
- Checagem leve de nomes/tipos sob demanda; casos ambíguos → erro pedindo
  anotação, como o Rust faz.
- **Interop Rust** (ver abaixo).

### 4. Interop com Rust

Estratégia híbrida — entrega "carregar Rust em runtime" sem pagar o custo do
cargo em todo run:

- **Stdlib + crates comuns:** funções nativas **registradas** no binário `alloy`
  (`cstd`, `http`, etc. expostas via funções Rust). Instantâneo, sem cargo.
- **Crates arbitrários:** carregados **sob demanda** via compilação de um shim
  `cdylib` com wrappers `extern "C"`, carregado com `libloading`. O shim
  compilado é **cacheado** (`~/.alloy/shims/`), então só o primeiro uso paga o
  `cargo build`; execuções seguintes reusam o `.dylib`/`.so`/`.dll`.
- Fronteira é **C-ABI**: tipos complexos (`String`, `Vec`, genéricos, traits)
  exigem superfície C-friendly; o wrapper é gerado quando possível, senão é erro
  de import explícito.

Justificativa registrada: Rust não tem ABI estável, então "carregar crate em
runtime" sempre passa por compilar um shim C-ABI. O cache torna isso aceitável;
a stdlib registrada evita o custo no caminho comum.

### 5. `bin/alloy`

- `alloy run f.crs` — interpreta direto.
- `alloy build f.crs` — binário self-contained (VM + bytecode embutido).
- `alloy repl` — REPL interativo.
- `alloy check f.crs` — só checagem de tipos/nomes, sem executar.

### 6. Ponte de paridade (CI)

Harness que roda cada `examples/copper/*.crs` em `alloy run` e `cforge run` e
compara stdout/exit. Divergência = falha de CI.

## Tratamento de erros

- **Compile-time da VM:** erros de tipo/nome viram diagnósticos com span
  (reusa a infra de erro recuperável da AST). `alloy check` expõe só isso.
- **Runtime:** panics da VM com **stack trace Copper** (linha/coluna do `.crs`),
  não stack trace Rust.
- **Shim/interop:** falha de `cargo build` do shim é reportada como erro de
  import, incluindo o output do cargo.

## Testes

- **Unit:** lowering AST→bytecode e execução por opcode.
- **Paridade (invariante-chave):** `examples/copper/*.crs` nos dois alvos;
  stdout/exit têm que bater. Roda no CI.
- **REPL/snapshot:** regressões de execução.

## Fases internas

Entrega "tudo no fim" (paridade ampla), mas estruturada para nunca ficar com um
sistema meio-quebrado sem nada rodando. Cada fase é um plano de implementação
próprio.

1. ~~**AST completa**~~ — **JÁ EXISTE** (`expr.rs` + `program.rs`). Pulada.
2. **VM núcleo (tree-walking):** avaliação de expr, vars, funcs, controle de
   fluxo, `println`. Crate `alloy-vm` + binário `alloy` com `run`. → primeiro
   `alloy run`. **(primeiro plano de implementação)**
3. **Tipos compostos:** struct/class/enum/impl/match/closures na VM.
4. **Genéricos + traits** na VM (e fechar gaps de `Raw` na AST sob demanda).
5. **Interop:** stdlib registrada + shim C-ABI cacheado sob demanda.
6. **`alloy build`** self-contained + `repl` + harness de paridade no CI.
7. **(futuro, opcional)** backend bytecode e/ou JIT Cranelift como otimização.

## Riscos

- **Divergência semântica** (roda na VM mas não compila como Rust): mitigada
  pela checagem leve estilo-Rust + harness de paridade no CI.
- **Escopo grande:** mitigado pelas fases internas; cada fase tem um entregável
  executável.
- **Custo de manutenção da AST dupla:** mitigado porque a AST completa é
  compartilhada — beneficia transpiler e LSP, não é trabalho jogado fora.

## Fora de escopo

- Forkar o `rustc`.
- Borrow-checker estático completo na VM.
- JIT (a VM começa como interpretador; JIT é trabalho futuro possível).
