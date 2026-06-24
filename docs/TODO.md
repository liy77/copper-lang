# Copper Lang — Backlog / Roadmap

> Última atualização: 2026-06-24. Itens ordenados por área e prioridade estimada.

---

## 🔴 Alta prioridade

### Alloy — Wasm interop: ABI rica (strings / Vec / structs)
- **Estado:** protótipo só suporta `i64` / `bool` escalares; `extern "C"` obrigatório.
- **O que falta:**
  - Fase 2 (especificada em `docs/superpowers/specs/2026-06-22-alloy-wasm-interop-design.md`):
    `f64`, strings, `Vec<T>`, slices via guest memory + alloc/free ABI (`wasm32-wasip1` + WASI no `wasmi::Linker`).
  - Fase 3: structs, `Option`/`Result`, traits via WebAssembly Component Model + `wit-bindgen`.
  - **Ergonomia:** auto-gerar wrapper de export para `pub fn` idiomático (sem `extern "C"`),
    para que o usuário não precise anotar manualmente.
- **Arquivo:** `crates/alloy-vm/src/wasm.rs`, `loader.rs`

### MUI codegen — controle de fluxo estrutural reativo (`if`/`for`/`match`)
- **Estado:** design aprovado, pré-implementação (`docs/superpowers/specs/2026-06-17-mui-structural-codegen-design.md`).
- **O que falta:**
  - `Node::If` → avaliar condição + assinalar signal; hoje sempre renderiza o branch `then`.
  - `Node::For` → iterar a lista; hoje renderiza exatamente uma iteração.
  - `Node::Match` → implementar arms; hoje o parser já descarta os arms (`arms: Vec::new()`);
    precisa: (1) parsear arms em `mui-syntax`, (2) emitir subscriptions em `mui-codegen`.
  - Rebuild whole-view reativo quando signal muda (modelo já validado no `mui-runtime`).
- **Arquivos:** `crates/mui-codegen/src/lib.rs:435-447`, `crates/mui-syntax/src/lib.rs:946`

### Transpilador — genéricos em parâmetros de função e structs (`func<T> name(x: T)`)
- **Estado:** não suportado. Apenas genéricos em tipo de retorno e em `struct` funcionam.
- **O que falta:** capturar `<T, U, ...>` após o nome da função em `parse_function` e emitir
  `fn name<T, U>(...) -> Ret` corretamente.
- **Arquivo:** `crates/copper-parser/src/parser/mod.rs` (`parse_function`)

---

## 🟡 Média prioridade

### Alloy — `&&` / `||` com short-circuit
- **Estado:** ambos os operandos são pré-avaliados antes de `eval_binary`
  (documentado em `CLAUDE.md` como limitação conhecida).
- **O que falta:** na avaliação de `BinOp::And`/`BinOp::Or` em `interp.rs`,
  usar avaliação preguiçosa do lado direito.
- **Arquivo:** `crates/alloy-vm/src/interp.rs` (`eval`)

### Alloy GUI — features pendentes do alloy-gui
- **Estado:** playground responsivo funcional, hot-reload, file dialog.
- **O que falta (ver `docs/superpowers/specs/2026-06-22-alloy-gui-design.md`):**
  - Output streaming (linha a linha conforme o programa imprime, não tudo de uma vez).
  - Histórico de arquivos recentes.
  - Painel de erros estruturado (vs. texto puro no output).
  - Packaging: `.app` bundle (macOS), `.exe` com ícone (Windows) via `make-app.sh`.

### copper-lsp — features LSP faltantes
- **Estado:** hover, completion, goto-definition funcionam.
- **O que falta:**
  - `inlayHints` — tipos inferidos de variáveis `let x = expr`.
  - `codeAction` — quick-fix `import { X } from cstd` quando X não resolvido.
  - `rename` — renomear símbolo em todos os usos no arquivo.
  - `formatting` — integrar `rustfmt` no Copper (ou formatar via regras próprias).
  - Diagnósticos de erros de tipo básicos (hoje só erros de parse).

### mui-lsp — features LSP faltantes
- **Estado:** hover, completion, goto-definition, documentColor funcionam (975 linhas).
- **O que falta:**
  - `inlayHints` — mostrar o tipo inferido de `signal(...)`.
  - `codeAction` — auto-importar componente de outro arquivo.
  - `rename` — renomear componente (view) em todos os usos.
  - Diagnósticos para props inválidas / tipos incompatíveis.

### Transpilador — aliases de tipo (`type Foo = Bar<T>`)
- **Estado:** aliases dentro de `static`/`as` casts não loweram corretamente (mencionado em `CLAUDE.md`).
- **O que falta:** garantir que `type Foo = ...` emite `type Foo = ...;` em Rust
  e que aliases Copper (ex: `int` → `i64`) funcionam dentro de aliases de tipo.

---

## 🟢 Baixa prioridade / polish

### Alloy — `scope.rs` e `scope_manager.rs` no transpilador
- Presentes mas praticamente vazios. Necessários para análise de variáveis livres em closures,
  shadowing correto e warnings de variável não usada. Pré-requisito para um type-checker leve.

### Alloy — `alloy check` Miri: melhorar UX
- Funciona, mas a saída do Miri é crua. Filtrar e formatar os erros de Miri
  para apresentar na mesma linguagem de erros do Alloy.

### installer-gui — Windows
- `installer.mui` existe, mas o teste em Windows ainda não foi feito end-to-end.
- Verificar `PATH` edit via `winreg` + `WM_SETTINGCHANGE` no installer real.

### scripts/release-alloy.py — artefatos de release
- Gera binários cross-platform; verificar se o signing (macOS notarização) está
  contemplado ou se precisa de um passo extra.

### Documentação do usuário
- `docs/INSTALL.md` e `README.md` não documentam `alloy` nem `alloy-gui` ainda.
- Adicionar seção de "Quick Start com Alloy" e referência dos comandos (`alloy run`,
  `alloy build`, `alloy check`).

### Generics turbofish em expressões (`foo::<T>()`)
- Mencionado no spec de MUI codegen como "non-goal por agora — merece spec próprio".
- Necessário para o north-star "tudo que Rust faz, Copper faz".

---

## ✅ Concluído recentemente (referência)

| Item | Commit |
|---|---|
| Alloy: Rust via wasm embutido (i64/bool, `extern "C"`) | `414cee0` |
| Alloy: `cstd` interpretado de `std/cstd.crs` (single source) | `99ad0b9` |
| alloy-gui: playground responsivo (output à direita) | `8b52315` |
| alloy-gui: MUI GUI angelic-theme, hot-reload, file dialog | `7d4fe67` |
| alloy: checagem de tipo de retorno no interpreter | `fcbc0e8` |
| alloy: `alloy check` / Miri auto-provisionado | `551f298` |
| alloy: imports locais `.crs` (merge + bundle no .loy) | `2f14a38` |
| alloy: artefato portável `.loy` + `alloy build` | `a726046` |
| copper-syntax: `mut (a,b) = ...` tuple destructuring | `0c96577` |
| i18n: comentários e mensagens em inglês | `0926b65` |
