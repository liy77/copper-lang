# Alloy GUI + dual-mode binary

**Data:** 2026-06-22
**Status:** Design aprovado (pré-implementação)
**Depende de:** [Alloy VM MVP](2026-06-22-alloy-vm-design.md) (concluído)

## Resumo

Transformar `alloy` num binário **dual-mode** distribuível, construído com
**MUI** (a UI declarativa do mocida), sem quebrar a portabilidade do crate
`alloy-vm`:

- **CLI:** `alloy run arquivo.crs` (e `alloy <arquivo.crs>`) interpreta no
  terminal — comportamento atual, reaproveitado.
- **GUI (sem args, ou `alloy gui`):** abre uma janela MUI com:
  - **Playground:** editor de `.crs` + botão Run + painel de saída.
  - **File runner:** abrir um `.crs` do disco e rodar.
  - **Atualizador:** checar/baixar/instalar nova versão do próprio `alloy`.

## Decisão de arquitetura: crate host separado

O `CLAUDE.md` do copper-lang exige que o caminho portátil **nunca linke
mocida** (senão quebra CI Linux/macOS). Portanto:

- `crates/alloy-vm` **permanece portátil** (lib + lógica de interpretação, sem
  mocida). Já está pronto.
- Crate novo **`alloy-gui`** (fora do workspace portátil, como o
  `installer-gui`) linka `mocida` + `mui-runtime` + `mui-syntax` e usa
  `alloy-vm` como lib. **Este** é o binário `alloy` de release/distribuição.
- O binário `alloy` do `alloy-gui` é o dual-mode: parseia args; com `run`/arquivo
  → chama a lib `alloy-vm`; sem args → sobe a GUI MUI.

Assim entregamos "um binário usável como GUI e CLI" sem sujar o CI. O binário
`alloy` do crate `alloy-vm` continua existindo para o CI portátil (testes), mas
a distribuição usa o de `alloy-gui`.

```
crates/alloy-vm  (lib + CLI portátil, sem mocida)   ← CI
        │ (path dep)
        ▼
alloy-gui/  (host: mocida + mui-runtime + mui-syntax + alloy-vm)
   ├── alloy.mui        view declarativa (playground/runner/updater)
   ├── backend.rs       lógica do host: rodar fonte, listar arquivos, update
   ├── app.bundle       nome/id + assets (ícone)
   ├── assets/          alloy-icon.png / alloy-icon.ico / alloy-logo.png
   ├── build.rs         embute .ico no .exe (winresource, só Windows)
   ├── packaging/        script .app/.icns para macOS
   └── src/main.rs      dual-mode (CLI passthrough + host MUI)
```

## Pré-requisito no `alloy-vm`: captura de saída

Hoje `println`/`print` escrevem direto em `stdout` via `println!`. A GUI precisa
**capturar** a saída para mostrar no painel. Refactor testável:

- `Interpreter` ganha um **sink de saída** configurável: por padrão escreve em
  stdout (preserva o comportamento da CLI); a GUI injeta um buffer.
- API proposta: `Interpreter` guarda `out: Box<dyn FnMut(&str)>` ou um enum
  `Output { Stdout, Buffer(Rc<RefCell<String>>) }`. `println`/`print` passam a
  escrever no sink. Builder: `Interpreter::with_output(sink)`; `new()` mantém
  stdout.
- Os testes existentes que dependem de stdout continuam válidos; novos testes
  capturam num buffer e asseguram o conteúdo.

Esta é a **primeira tarefa** (no crate portátil, testável no CI).

## Componentes do `alloy-gui`

### `src/main.rs` — dual-mode
- `args`: se o primeiro arg for `run` ou um caminho `.crs` existente → modo CLI
  (chama `alloy_vm::run_source(path)`); `--update` → roda updater headless;
  nenhum arg / `gui` → sobe a GUI.
- Modo GUI: padrão de host do `mui-dev` (parse `alloy.mui` → `entry_view` →
  `window_config` → `prefer_renderer`/`prefer_custom_titlebar` → `App::new` →
  `build_view_seeded` → seed de signals → `app.set_children` → `on_tick`
  (rebuild em `take_dirty`, polling de resultados do backend) → `show().run()`).
- Ícone da janela: `app.bundle` registra `mocida://alloy-icon.png` +
  `app.set_window_icon("mocida://alloy-icon.png")`.

### `backend.rs` — lógica do host (std + alloy-vm)
- `run_source(src: &str) -> RunResult { output: String, error: Option<String> }`
  — parseia via `copper_syntax::program::parse_program` e interpreta com
  `Interpreter::with_output(buffer)`, devolvendo texto capturado + erros
  (sintaxe/runtime) formatados com span.
- `read_file(path)` / `list_dirs(prefix)` — file runner + autocomplete.
- `check_update()` / `apply_update()` — atualizador (ver abaixo).

### `alloy.mui` — view
- Abas/seções: **Playground** (TextArea editável + Run + Output), **Runner**
  (campo de caminho + autocomplete + Run), **Update** (versão atual, "Verificar",
  progresso). Signals dirigidos pelo host (`source`, `output`, `err_lines`,
  `update_status`, `progress`).

### Atualizador
- `check_update()`: consulta a fonte de releases (a definir — GitHub releases ou
  endpoint), compara com a versão embutida (`env!("CARGO_PKG_VERSION")`).
- `apply_update()`: baixa o binário da plataforma, substitui o executável atual
  (self-replace: baixa para temp, troca no restart), reporta progresso via
  signals. Detalhes da fonte de download ficam para a fase de implementação.

### Ícones (ambas plataformas)
- **Janela (runtime):** `app.bundle` + `set_window_icon` — ambas plataformas.
- **`.exe` (Windows):** `build.rs` com `winresource` embute `assets/alloy-icon.ico`
  no PE (padrão novo no repo; sob `#[cfg(windows)]`). Requer gerar o `.ico` do
  PNG (script de packaging).
- **`.app` (macOS):** script `packaging/make-app.sh` que gera `Alloy.icns` do PNG
  (via `iconutil`/`sips`) e monta `Alloy.app` (Info.plist + binário + `.icns` +
  `libmocida.dylib`). Usável no Finder e ainda invocável por linha de comando
  (`Alloy.app/Contents/MacOS/alloy run x.crs`).

## Assets necessários (fornecidos pelo usuário)
- `assets/alloy/alloy-logo.png` — sem caixa, para README (já referenciado).
- `assets/alloy/alloy-icon.png` — com caixa de app, fonte do `.ico`/`.icns` e do
  ícone de janela. **Bloqueio atual:** arquivos PNG precisam ser salvos em disco.

## Testes / verificação
- **`alloy-vm` (CI):** testes de captura de saída (buffer) + os existentes.
- **`alloy-gui`:** não linka no CI portátil (precisa de `MOCIDA_LIB_DIR`); a
  lógica de `backend.rs` que não depende de mocida (`run_source`, `read_file`,
  parsing de versão do updater) é testável isoladamente. A GUI em si **não é
  testada neste ambiente** (decisão do usuário) — só escrita.

## Fora de escopo (por ora)
- Hot-reload do `.mui` no host de release (é coisa do `cforge run`/`mui-dev`).
- REPL com estado incremental (a VM executa um `Program` inteiro hoje).

## Fases internas
1. **Captura de saída no `alloy-vm`** (testável, CI). ← primeiro
2. **Crate `alloy-gui`**: scaffold + `main.rs` dual-mode (CLI passthrough já
   funciona) + `backend::run_source` (testável sem mocida).
3. **`alloy.mui`** + wiring de signals (playground + runner).
4. **Atualizador** (`check_update`/`apply_update`).
5. **Ícones**: `build.rs` winresource (Windows) + `packaging/make-app.sh`
   (macOS) + `app.bundle`/ícone de janela.
