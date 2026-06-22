# Alloy feature-complete — rodar TODOS os examples/copper/*.crs

> **For agentic workers:** executado via subagent-driven-development. Os EXEMPLOS são o critério de aceitação: cada cluster termina com `alloy run` executando os exemplos-alvo sem gaps de linguagem.

**Goal:** Estender `crates/alloy-vm` para que todo `examples/copper/*.crs` rode pelo `alloy run` como o `cforge` rodaria.

**Acceptance:** `alloy run <ex>` executa cada exemplo. Para exemplos puramente locais (ternary, tuples, optional, methods, matching, collections, unsafe, url, cstd, fs, time, json, crypto): exit 0 + saída coerente. Para os que dependem de rede/servidor (http, net, ws): o interpretador executa as chamadas nativas (sem "construção não suportada"/"função não definida"); falha de rede/servidor é ambiental e aceitável.

## Estratégia

Dois eixos:
- **Linguagem** (`interp.rs` + `value.rs`): expandir o modelo de valores e o `eval_expr`/`exec_stmt` para cobrir os construtos compostos.
- **Stdlib nativa** (`crates/alloy-vm/src/std_native/`): funções Rust registradas, chamadas por nome quando o exemplo importa de `cstd`/`fs`/`time`/`net`/`url`/`ws`/`json`/`crypto`/`http`. O host de import resolve `import { f } from mod` → registra `mod::f` na tabela de builtins.

## Modelo de valores (value.rs) — expansão cross-cutting (FAÇA PRIMEIRO)

Adicionar a `enum Value`:
- `Vec(Rc<RefCell<Vec<Value>>>)` — arrays/vec, mutáveis e compartilháveis.
- `Tuple(Vec<Value>)` — tuplas.
- `Map(Rc<RefCell<IndexMap<String, Value>>>)` ou `Vec<(String,Value)>` — objetos/JSON.
- `Struct { name: String, fields: Rc<RefCell<HashMap<String,Value>>> }` — instâncias de struct/class.
- `Enum { ty: String, variant: String, payload: Vec<Value> }` — enums + `Option`(Some/None)/`Result`(Ok/Err) modelados como `Enum`.
- `Closure(Rc<ClosureData>)` — `params`, `body`, `captured env`.
- `Native(NativeFn)` — função nativa registrada (stdlib + builtins).

`Display`/`type_name`/`as_bool` atualizados. `PartialEq` para comparação em `match`/`==`.

## Clusters (cada um = uma ou mais tasks, testadas contra exemplos)

### Cluster 1 — Coleções e acesso (desbloqueia: collections parcial, base de tudo)
- `ExprKind::Array` → `Value::Vec`.
- `ExprKind::Index` → indexação de Vec/Map (e `body["a"]["b"]`).
- `ExprKind::Member` (field access) → `Struct`/`Map`/tupla `.0`.
- `vec![...]` macro → `Value::Vec`.
- Acceptance parcial: indexação e arrays avaliam.

### Cluster 2 — Tuplas (desbloqueia: tuples)
- Tuple literal `(a,b)`, indexação `.0`/`.1`, destructuring em `let`/`mut` (`(a,b) = ...`), tuplas aninhadas, função retornando tupla.
- Acceptance: `alloy run examples/copper/tuples.crs` → exit 0.

### Cluster 3 — Structs, classes, impl, métodos (desbloqueia: methods, optional, unsafe)
- `Item::Struct`/`Item::Class`/`Item::Impl` registrados; `Class` lowered p/ struct+métodos.
- `ExprKind::StructLit` → `Value::Struct`.
- `ExprKind::Path` (`Rect::new`) → função associada.
- Method dispatch: `obj.metodo(args)` resolve em impl do tipo; `self` no escopo.
- `?.` optional chaining; `as` cast.
- `unsafe { }` / `unsafe func` → executa o corpo ignorando `unsafe`; raw ptr/deref: stub seguro (modela `*p` como o valor apontado via `Ref`, ou trata `&x`/`*p` como identidade no nível interpretado).
- Acceptance: methods.crs, optional.crs, unsafe.crs → exit 0.

### Cluster 4 — Enums, Option/Result, match (desbloqueia: matching, collections)
- `Item::Enum`; `Some`/`None`/`Ok`/`Err` como construtores nativos → `Value::Enum`.
- `match` (literal, OR `a|b`, guard `if`, wildcard `_`, tuple-struct `Some(x)`), `if let`, `while let`.
- `?` try operator sobre `Result`/`Option`.
- Acceptance: matching.crs → exit 0.

### Cluster 5 — Closures + métodos de iterador built-in (desbloqueia: collections)
- `ExprKind::Closure` → `Value::Closure` com captura.
- Métodos built-in em `Value::Vec`/iteradores: `.iter()`, `.into_iter()`, `.map(cl)`, `.filter(cl)`, `.copied()`, `.cloned()`, `.collect()`, `.sum()`, `.next()`, `.len()`, `.is_empty()`, `.count()`, `.abs()`, `.recip()`, `.unwrap()`, `.parse::<T>()`, e métodos de String (`.to_string()`, `.to_uppercase()`, `.chars()`, `.len()`). Modelar `iter`/`map`/`filter` de forma eager (Vec→Vec) já basta.
- Acceptance: collections.crs → exit 0.

### Cluster 6 — Dispatch de stdlib + import (desbloqueia os com `import`)
- O parser de `Program` já dá `Item::Import { kind, path }`. No `load_program`, para cada import de módulo conhecido, registrar os símbolos importados como `Value::Native` apontando para as implementações Rust.
- Tabela de builtins por módulo em `std_native/`.

### Cluster 7 — stdlib pura/local em Rust (desbloqueia: cstd, fs, time, url, net, ws)
- `cstd`: input, readln, exit, sleep_ms, now_ms, args, env, exists, is_file, is_dir, read_file, write_file, etc. (std puro).
- `fs`: read/write/append/exists/mkdir/remove_dir/size/list/... (`std::fs`).
- `time`: now_secs/now_ms/now_nanos/sleep_ms/mono_ms/iso8601/now_iso (`std::time`).
- `url`: encode/decode/query2/join (string puro).
- `net`: resolve/is_port_open/tcp_request/local_ip (`std::net`).
- `ws`: request/send (RFC6455 sobre `std::net::TcpStream`) — porta direta do `std/ws.crs`.
- Acceptance: cstd/fs/time/url/net → executam (net pode falhar por ambiente). `input()` lê stdin; em ambiente sem tty, retornar string vazia em vez de travar.

### Cluster 8 — stdlib com crates (desbloqueia: json, crypto, http)
- Adicionar deps ao `alloy-vm`: `serde_json@1`, `sha2@0.10`, `hmac@0.12`, `ureq@2`.
- `json`: get/get_int/get_bool/has/len/pretty (serde_json) — e suportar indexação `Value` de json (`body["a"]["b"]`) integrando com `Value::Map`.
- `crypto`: base64/hex/crc32 (puro) + sha256/sha512/hmac_sha256 (sha2/hmac).
- `http`: get/post_json/download (ureq) retornando um `Value::Struct` "Response" com `.status`/`.body`/`.is_ok()`/`.json()`.
- Acceptance: json/crypto → exit 0; http → executa (rede ambiental).

## Notas
- **Sem panics:** todo gap remanescente continua sendo `RuntimeError` com span, nunca panic.
- **Portabilidade:** as deps adicionadas (serde_json/sha2/hmac/ureq) são puro-Rust; não quebram CI. alloy-vm continua sem mocida.
- **ws/http/net:** "rodar" = executar as chamadas; conectividade é ambiental.
- Cada cluster: implementar → `cargo test -p alloy-vm` + `alloy run` nos exemplos-alvo → fmt/clippy → commit.
