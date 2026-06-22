# Alloy GUI — host nativo + binário dual-mode

O binário **`alloy`** distribuível: funciona como **linha de comando** e como
**aplicativo gráfico** (GUI feita em MUI). Espelha o padrão do `installer-gui`:
a UI vive inteira em `alloy.mui`, e este crate é o host nativo que linka o
runtime mocida + o backend.

> Este crate **não** é membro do workspace do copper-lang (ele linka mocida, o
> que os crates portáteis não podem). Compile-o explicitamente.

## Modos

```sh
alloy run arquivo.crs     # interpreta no terminal (reusa o crate alloy-vm)
alloy arquivo.crs         # idem (caminho direto)
alloy update              # auto-atualização headless
alloy            # (sem args) abre a GUI
alloy gui                 # idem
```

A GUI tem três áreas: **Playground** (editor + Run + saída), **File runner**
(abrir um `.crs` e rodar) e **Update** (verificar/instalar nova versão).

## Layout

```
alloy-gui/
├── alloy.mui          A UI inteira (MUI declarativa). Signals dirigidos pelo host.
├── backend.rs         Lógica: run_source (usa alloy-vm), read_file, updater. std-only.
├── app.bundle         Nome/id + ícone (mocida://alloy-icon.png).
├── assets/            alloy-icon.png / alloy-icon.ico (ver assets/README.md).
├── build.rs           Embute o .ico no .exe (winresource, só Windows).
├── packaging/
│   └── make-app.sh    Gera Alloy.app + Alloy.icns no macOS.
└── src/main.rs        Dual-mode: CLI passthrough + host MUI.
```

## Pré-requisitos de build

O host linka o runtime mocida via os crates do workspace `mocida-rs`, que deve
estar como irmão deste repo:

```
<root>/copper-lang/        (este repo)
<root>/mocida/mocida-rs/   (runtime mocida)
```

E as variáveis do `mocida-sys` (caminho dos headers/lib nativa):

```sh
export MOCIDA_INCLUDE_DIR=/caminho/para/uikit/headers
export MOCIDA_LIB_DIR=/caminho/para/libmocida
```

## Build

```sh
# binário (GUI + CLI)
cargo build --release --manifest-path alloy-gui/Cargo.toml
# → alloy-gui/target/release/alloy   (+ libmocida.dylib/.dll ao lado)

# macOS: empacotar como Alloy.app (com ícone)
alloy-gui/packaging/make-app.sh
```

No Windows o `build.rs` embute `assets/alloy-icon.ico` no `.exe`. No macOS o
ícone vem do `Alloy.app` gerado pelo script.

## Estado

- **CLI**: completo (reusa `alloy-vm`).
- **GUI / backend / updater**: escritos contra a API do mocida e o padrão do
  installer; **ainda não compilados/testados contra o mocida real** neste
  ambiente. Ajustes podem ser necessários ao linkar (API de `Input` multiline,
  nomes de signals). O `REPO_SLUG`/fonte de releases no `backend.rs` é um
  `TODO` a confirmar.
