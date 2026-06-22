# Alloy `.loy` — artefato portátil + runtime cross-platform (modelo "JVM")

**Data:** 2026-06-22
**Status:** Design aprovado (pré-implementação)
**Depende de:** [Alloy VM](2026-06-22-alloy-vm-design.md) (interpretador completo)

## Objetivo

Tornar o Alloy "write once, run anywhere" como Java/JVM, em duas peças:

- **A — artefato portátil (`.loy`):** compila um `.crs` **uma vez** num arquivo
  binário independente de plataforma, que roda em qualquer `alloy` de qualquer
  SO, sem reenviar o fonte e sem recompilar (equivale ao `.jar`).
- **C — runtime distribuído:** o binário `alloy` cross-compilado e empacotado
  para Windows/macOS/Linux (equivale à JVM instalada por plataforma).

O "mesmo código roda igual em todo lugar" já vale hoje no nível de fonte (o
interpretador normaliza o comportamento). Este trabalho adiciona o artefato
compilado e garante o runtime em todas as plataformas.

## A — Formato `.loy` (AST serializada)

Decisão: o artefato é a **AST (`Program`) serializada**, não um bytecode de
pilha. Reaproveita 100% o interpretador tree-walking; a fronteira natural de
"compilação" é o `Program` já parseado. (Um bytecode de pilha + VM dedicada
fica como evolução futura — `A2` no brainstor — sem desperdiçar este trabalho.)

### Layout do arquivo

```
offset 0:  magic     = b"ALLOYBC\0"   (8 bytes)
offset 8:  fmt_ver    : u16 LE        (versão do formato; muda se a AST mudar)
           alloy_ver  : string        (versão do alloy gerador; bincode)
           items      : Vec<Item>     (o Program parseado; bincode)
```

- **Independente de plataforma:** é só dados (sem código nativo, sem endianness
  ambígua — bincode com config fixa LE).
- **Serialização:** `serde` + `bincode`. Deriva-se `Serialize`/`Deserialize`
  nos tipos de AST de `copper-syntax` consumidos pelo `Program`:
  `Span`, `Param`, `Field`, `ClassMember`, `ImportKind`, `Item`, `Block`,
  `Stmt`, `Expr`, `ExprKind`, `Literal`, `StrTemplate`, `StrPart`, `BinOp`,
  `UnOp`, `AssignOp`, `Pattern`, `MatchArm`, `Type`. (Não afeta a transpilação
  do `cforge`, que ignora a AST.)
- **`bincode`** entra como dependência só do `alloy-vm` (puro-Rust).

### Módulo `crate::bytecode` (em `alloy-vm`)

```rust
pub const MAGIC: &[u8; 8] = b"ALLOYBC\0";
pub const FMT_VERSION: u16 = 1;

/// Serializa um Program em bytes `.loy` (header + bincode dos itens).
pub fn compile(items: &[Item]) -> Vec<u8>;

/// `true` se os bytes começam com o magic do formato.
pub fn is_bytecode(bytes: &[u8]) -> bool;

/// Verifica magic + versão e desserializa os itens. Erro claro em mismatch.
pub fn load(bytes: &[u8]) -> Result<Vec<Item>, String>;
```

`load` reconstrói `Program { items, errors: vec![] }` para alimentar o
`Interpreter::run_program` existente, sem mudar o interpretador.

### Comandos

- **`alloy build app.crs [-o app.loy]`** — parseia; em sucesso grava o
  `.loy` (saída default: mesmo stem + `.loy`). Erros de sintaxe →
  stderr + exit 1. Nada é executado.
- **`alloy run <arquivo>`** — lê os bytes; se `is_bytecode` → `load` + interpreta;
  senão trata como fonte `.crs`. Logo `alloy run app.loy` roda em qualquer SO.
- **`cforge vm build app.crs`** — passa a **emitir `.loy`** (antes só
  validava), e **`cforge vm run`** também detecta o formato. Mantém os aliases
  (`virtual`).
- **Erros de artefato:** magic inválido/arquivo truncado → "arquivo .loy
  inválido"; `fmt_ver` diferente de `FMT_VERSION` → "artefato gerado por outra
  versão do Alloy (formato vX, runtime espera vY)". Ambos exit 1, sem panic.

### Self-contained

A stdlib (`cstd`/`fs`/`http`/`json`/…) é resolvida **nativamente em runtime**
pelo interpretador, então o `.loy` não embute o fonte dela — basta o
runtime `alloy`. Projetos **multi-arquivo** com `import` local de `.crs`/`.rs`
ficam fora de escopo aqui (os exemplos são single-file); evolução futura:
bundlar múltiplos `Program` no artefato.

## C — Distribuição cross-platform do runtime

O binário `alloy` está no crate portátil `alloy-vm` (sem mocida; deps
`ureq`+rustls / `serde_json` / `sha2` / `hmac` / `bincode` são puro-Rust),
então cross-compila limpo.

- **Script `scripts/release-alloy.py`** (Python, como o resto de `scripts/`):
  para cada alvo disponível, roda
  `cargo build --release --target <T> -p alloy-vm --bin alloy` e empacota o
  binário (+ `README`/licença) em `dist/alloy/alloy-<versão>-<T>.{tar.gz|zip}`.
- **Alvos:** `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`,
  `x86_64-apple-darwin`, `aarch64-apple-darwin`, `x86_64-pc-windows-msvc`,
  `aarch64-pc-windows-msvc`. O script **pula** alvos cujo toolchain não está
  instalado (com aviso), em vez de falhar — útil pra rodar localmente.
- **Versão:** derivada de `CARGO_PKG_VERSION` do `alloy-vm`.
- Instalador por-SO fica como passo opcional futuro.

## Testes

- **Round-trip (CI):** para cada `examples/copper/*.crs`,
  `alloy build` → `.loy` → `alloy run app.loy` produz **a mesma saída**
  que `alloy run app.crs`. (Harness compara stdout/exit.)
- **Unit:** `compile`/`load` round-trip de um `Program` em memória; `is_bytecode`
  positivo/negativo.
- **Erros:** magic inválido e `fmt_ver` incompatível retornam `Err` claro (sem
  panic).
- **Cross-compile (best-effort):** o script reporta quais alvos buildaram; não
  é gate de CI (depende dos toolchains instalados).

## Fora de escopo
- Bytecode de pilha / JIT (evolução futura).
- Projetos multi-arquivo num único `.loy`.
- Assinatura/compressão do artefato.
