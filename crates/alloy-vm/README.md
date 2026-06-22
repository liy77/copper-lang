<p align="center">
  <img src="../../assets/alloy/alloy-logo.png" alt="Alloy" width="180">
</p>

<h1 align="center">Alloy</h1>

<p align="center">
  <em>A VM de iteração rápida do Copper — interpretador tree-walking.</em>
</p>

---

## O que é

**Alloy** é o interpretador do [Copper](../../README.md): um binário separado
(`alloy`) que **executa `.crs` direto**, sem transpilar para Rust nem chamar o
`cargo`. Ele anda sobre a mesma AST que o transpiler de produção (`cforge`)
usa — então o que roda no Alloy é o mesmo Copper que compila nativo.

Dois alvos, uma linguagem:

| Ferramenta | Caminho | Para quê |
| --- | --- | --- |
| `cforge` | `.crs` → Rust → `cargo build` | build nativo de release |
| **`alloy`** | `.crs` → AST → interpretação | iteração rápida, scripting, REPL |

A compatibilidade com Rust é a invariante central: **o mesmo `.crs` produz o
mesmo comportamento** nos dois caminhos.

## Uso

```sh
alloy run programa.crs
```

Exemplo (`examples/copper/alloy-hello.crs`):

```rust
func int soma_ate(n: int) {
    mut total = 0
    for i in 1..n {
        total += i
    }
    return total
}

func main() {
    println("Alloy VM")
    mut s = soma_ate(5)
    println("soma 1..5 = ${s}")
}
```

```sh
$ alloy run examples/copper/alloy-hello.crs
Alloy VM
soma 1..5 = 10
```

## Suportado hoje (MVP)

- Literais escalares (`int`, `float`, `bool`, `str`) e interpolação `"${expr}"`.
- Operadores binários/unários, ternário, atribuição (`=`, `+=`, …), `++`/`--`.
- Controle de fluxo: `if`/`else`, `while`, `loop`, `for x in a..b`,
  `break`/`continue`.
- Funções do usuário, chamadas e recursão.
- Builtins `println` / `print`.

Aritmética de inteiros é **checada** (overflow e divisão/resto por zero viram
erro de runtime, nunca panic). Construções fora do subset do MVP
(struct/enum/impl/match/closures, genéricos, traits) reportam um erro de
runtime claro — estão no roadmap.

## Roadmap

Veja o design completo em
[`docs/superpowers/specs/2026-06-22-alloy-vm-design.md`](../../docs/superpowers/specs/2026-06-22-alloy-vm-design.md):

1. ✅ Núcleo tree-walking (expr, vars, controle de fluxo, funções, `println`).
2. Tipos compostos: struct/class/enum/impl/match/closures.
3. Genéricos + traits.
4. Interop com Rust: stdlib registrada + shim C-ABI cacheado sob demanda.
5. `alloy build` (binário self-contained) + `alloy repl`.
6. *(futuro)* backend de bytecode e/ou JIT Cranelift como otimização.

## Arquitetura

```
crates/copper-syntax::program  (Program AST — corpos parseados)
            │
            ▼
crates/alloy-vm                bin: alloy
(interpretador tree-walking)   (run / build / repl / check)
```

- `value.rs` — valores em runtime (`Value`).
- `env.rs` — escopos encadeados (`Env`).
- `error.rs` — `RuntimeError` + fluxo de controle.
- `interp.rs` — o interpretador (`Interpreter`).
- `src/bin/alloy.rs` — o CLI.

O crate é **isolado**: `cforge` e os crates `mui-*` não dependem dele, então o
CI multiplataforma não é afetado.
