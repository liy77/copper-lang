# `alloy check` / `cforge check` — verificação Rust (borrow + Miri) auto-provisionada

**Data:** 2026-06-22
**Status:** Design aprovado (pré-implementação)
**Depende de:** Alloy VM + `cforge` (transpiler)

## Objetivo

Dar ao usuário a **verificação real do Rust** (type check + borrow checker + UB
via Miri) para um `.crs`, **sem ele instalar nada manualmente** e sem carregar
o rustc dentro do binário do Alloy. O `alloy run` continua dinâmico/instantâneo;
`check` é o passo opcional que confirma que o programa respeita as regras do
Rust (borrows, lifetimes, etc.) — fechando a lacuna do interpretador.

## Por que não embutir o Miri no binário

Miri é construído contra as APIs internas do rustc (`rustc_private`, nightly).
Embuti-lo = vendorizar um rustc inteiro (centenas de MB, nightly pinado,
manutenção de fork). Em vez disso, **auto-provisionamos** o toolchain real e o
usamos como verificador — o usuário não digita comando de instalação.

## Componentes

### 1. Onde mora
- **`cforge check <file.crs>`** faz o trabalho: transpila (reusa o pipeline de
  `cforge::compile` + `generate_toml` que já gera `dist/rust/`) e roda o Miri
  sobre o crate gerado.
- **`alloy check <file.crs>`** **delega** ao `cforge check` (mesmo mecanismo de
  `delegate_to_cforge` já usado para imports `.rs`). Localiza o `cforge` ao lado
  do `alloy` ou no PATH.

### 2. Auto-provisionamento do toolchain (`src/cforge/toolchain.rs`)
- Usa um **rustup isolado**: `RUSTUP_HOME = ~/.alloy/rustup` (não mexe no rustup
  do usuário). `CARGO_HOME` pode ficar o padrão.
- `ensure_miri() -> Result<String, String>`:
  1. Se `rustup` não está no PATH → erro claro ("instale o rustup uma vez:
     https://rustup.rs") — é o único pré-requisito, e é o instalador padrão de
     Rust.
  2. Garante o toolchain nightly: `rustup toolchain install nightly
     --profile minimal` (idempotente; só baixa na 1ª vez).
  3. Garante o componente: `rustup component add miri --toolchain nightly`.
  4. Retorna o nome do toolchain (`nightly`).
- Tudo com `RUSTUP_HOME` apontando para `~/.alloy/rustup`, então o download
  (~centenas de MB) fica cacheado lá e some o "instale o miri" manual.
- Mensagens de progresso (`baixando toolchain de verificação…`) na 1ª vez.

### 3. Execução da verificação
- Após transpilar para `dist/rust/`, roda no diretório do crate:
  `cargo +nightly miri run` com `RUSTUP_HOME=~/.alloy/rustup`.
  - Miri faz type check + **borrow check** (frontend do rustc) e interpreta a
    MIR detectando UB.
  - Saída/erros do Miri são repassados; exit code propagado.
- **Modo rápido opcional** `--no-miri`: só `cargo +nightly check` (type+borrow,
  sem interpretar) — mais rápido quando o usuário só quer o veredito de borrow.

### 4. Fluxo do usuário
```
alloy check app.crs      # delega → cforge check
cforge check app.crs     # transpila + provisiona (1ª vez) + roda miri
cforge check app.crs --no-miri   # só borrow/type check (cargo check)
```

## Erros / casos
- `rustup` ausente → mensagem única e clara (instalar rustup; é o único passo
  manual, igual instalar a JVM uma vez).
- Falha de transpile → erros do cforge, exit 1.
- Miri/borrow falha → repassa a saída do cargo/miri, exit ≠ 0 (é o ponto: pegar
  o erro de borrow).
- Sem rede na 1ª provisão → erro do rustup repassado.

## Testes
- **Unit:** construção dos comandos (`ensure_miri` monta os args corretos;
  `check` monta `cargo +nightly miri run` com o `RUSTUP_HOME` certo) — testável
  sem baixar nada (injetar um "runner" fake / testar a montagem de `Command`).
- **Integração (best-effort, fora do CICD por padrão):** num ambiente com rede,
  `cforge check examples/copper/loops.crs` provisiona e roda; um exemplo com
  violação de borrow deliberada retorna erro. Marcado `#[ignore]` (depende de
  baixar nightly+miri).
- **Limite honesto:** o download do nightly (~centenas de MB) não roda no CI
  nem foi exercitado no ambiente de desenvolvimento; a lógica é testada por
  unidade e o caminho real roda na máquina do usuário.

## Fora de escopo
- Embutir o rustc/Miri no binário (fork) — evolução futura se desejado.
- Cross-check automático em todo `alloy run` (check é explícito/opcional).
