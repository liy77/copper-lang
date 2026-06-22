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
                         crates/copper-syntax   (AST — a completar)
                                   │
                  ┌────────────────┴────────────────┐
                  ▼                                  ▼
        crates/alloy-compiler                 crates/copper-parser
        (AST → bytecode Alloy)                (AST/tokens → Rust source)  ← já existe
                  │                                  │
                  ▼                                  ▼
        crates/alloy-vm                          cargo build
        (interpretador + runtime)              (binário nativo, release)
                  │
                  ▼
        bin: alloy   (run / build / repl / check)
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

### 1. `copper-syntax` (estender)

A AST hoje é Phase 1 (corpos de função capturados por *span*, não em árvore).
Precisa virar árvore completa:

- `Expr`: literais, binário/unário, chamadas, index, closures, match,
  interpolação de string, `?` (try), `?.` (optional chain), ternário.
- `Stmt`: `let`/`mut`, atribuição, loops (`loop`/`while`/`for`), `if`/`else`,
  `return`/`break`/`continue`, expr-stmt.
- Itens: `struct`/`enum`/`impl`/`trait`/`func` com genéricos.

Mantém-se **recuperável** (já é) para o LSP renderizar diagnósticos sobre árvore
parcial. Completar a AST é o **pré-requisito de todo o resto** e beneficia
também o transpiler e o LSP.

### 2. `alloy-compiler` (AST → bytecode)

- Lowering da AST para **bytecode de pilha** (stack-based — simples de
  implementar e depurar).
- Resolução de nomes/escopos, layout de structs/enums, tabela de funções.
- **Tipagem leve estilo-Rust**: o suficiente para que divergência com o `rustc`
  seja rara. Não é um borrow-checker estático completo. Casos ambíguos → erro
  pedindo anotação, como o Rust faz.

### 3. `alloy-vm` (runtime)

- Loop de execução do bytecode: pilha de valores + frames de chamada.
- Modelo de valores: `Int`/`Float`/`Bool`/`Str`/`Vec`/`Map`/`Struct`/`Enum`/
  `Closure`/`Ref`.
- **Memória:** ownership/move/borrow **simulados em runtime** (não estáticos).
  Move invalida a origem; borrows checados dinamicamente. Mantém a semântica
  Rust *observável* sem reimplementar o borrow-checker.
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
sistema meio-quebrado sem nada rodando:

1. **AST completa** (`Expr`/`Stmt`/itens) — pré-requisito de tudo.
2. **VM núcleo:** expr, vars, funcs, controle de fluxo. → primeiro `alloy run`.
3. **Tipos compostos:** struct/enum/impl/match/closures.
4. **Genéricos + traits.**
5. **Interop:** stdlib registrada + shim C-ABI cacheado sob demanda.
6. **`alloy build`** self-contained + harness de paridade no CI.

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
