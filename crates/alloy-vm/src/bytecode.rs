//! Formato `.loy`: a AST (`Program`) serializada num artefato binário
//! independente de plataforma — o "compila uma vez, roda em qualquer `alloy`".
//!
//! Layout: `magic(8)` + `fmt_ver: u16 LE` + bincode(`alloy_ver: String`) +
//! bincode(`items: Vec<Item>`). É só dados (sem código nativo), então o mesmo
//! arquivo roda em Windows/macOS/Linux.

use copper_syntax::program::{Item, Program};

pub const MAGIC: &[u8; 8] = b"ALLOYBC\0";
/// Versão do formato. Incremente quando a forma da AST mudar de modo
/// incompatível (novos campos/variantes que quebram a desserialização).
pub const FMT_VERSION: u16 = 1;

const ALLOY_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Serializa os itens de um `Program` em bytes `.loy`.
pub fn compile(items: &[Item]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&FMT_VERSION.to_le_bytes());
    // versão geradora (informativa) + itens.
    let ver = bincode::serialize(ALLOY_VERSION).unwrap_or_default();
    out.extend_from_slice(&ver);
    let body = bincode::serialize(items).unwrap_or_default();
    out.extend_from_slice(&body);
    out
}

/// `true` se os bytes começam com o magic do formato `.loy`.
pub fn is_bytecode(bytes: &[u8]) -> bool {
    bytes.len() >= 8 && &bytes[..8] == MAGIC
}

/// Verifica magic + versão e desserializa o `Program`. Nunca panica: devolve
/// `Err` com mensagem clara em qualquer inconsistência.
pub fn load(bytes: &[u8]) -> Result<Program, String> {
    if !is_bytecode(bytes) {
        return Err("arquivo .loy inválido (magic ausente)".into());
    }
    if bytes.len() < 10 {
        return Err("arquivo .loy truncado".into());
    }
    let ver = u16::from_le_bytes([bytes[8], bytes[9]]);
    if ver != FMT_VERSION {
        return Err(format!(
            "artefato gerado por outra versão do Alloy (formato v{ver}, runtime espera v{FMT_VERSION})"
        ));
    }
    let mut rest = &bytes[10..];
    // pula a string de versão geradora.
    let _gen_ver: String =
        bincode_take(&mut rest).map_err(|e| format!("artefato corrompido: {e}"))?;
    let items: Vec<Item> =
        bincode::deserialize(rest).map_err(|e| format!("artefato corrompido: {e}"))?;
    Ok(Program {
        items,
        errors: Vec::new(),
    })
}

/// Desserializa um valor do início de `buf` e avança `buf` para depois dele.
fn bincode_take<T: serde::de::DeserializeOwned + serde::Serialize>(
    buf: &mut &[u8],
) -> Result<T, bincode::Error> {
    use bincode::Options;
    // Mede quanto foi consumido reserializando o valor lido (bincode 1 não
    // expõe o cursor diretamente; a config default é determinística).
    let opts = bincode::options()
        .with_fixint_encoding()
        .allow_trailing_bytes();
    let value: T = opts.deserialize(buf)?;
    let consumed = bincode::options()
        .with_fixint_encoding()
        .serialized_size(&value)? as usize;
    *buf = &buf[consumed..];
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use copper_syntax::program::parse_program;

    #[test]
    fn round_trip() {
        let prog = parse_program("func int add(a: int, b: int) { return a + b }\nadd(2, 3)\n");
        let bytes = compile(&prog.items);
        assert!(is_bytecode(&bytes));
        let loaded = load(&bytes).expect("load ok");
        assert_eq!(loaded.items.len(), prog.items.len());
    }

    #[test]
    fn rejects_garbage_and_bad_version() {
        assert!(!is_bytecode(b"not bytecode"));
        assert!(load(b"not bytecode at all").is_err());
        let mut bytes = compile(&[]);
        bytes[8] = 99; // corrompe a versão
        assert!(load(&bytes).is_err());
    }
}
