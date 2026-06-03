//! Curated method tables for Rust stdlib types that surface in Copper code:
//! `str` / `String`, `Vec`, `Option`, `Result`, `HashMap`, `i32`/`i64`/`f64`,
//! `bool`, `char`. Drives `obj.|` completion when the receiver's inferred
//! type is one of these (or unknown — see `union_methods` for the fallback).
//!
//! Hand-curated rather than scraped from rustdoc because the long tail of
//! std methods is overkill for a transpiler-targeted helper. Add entries
//! when a method becomes idiomatic in Copper code.

use tower_lsp::lsp_types::CompletionItemKind;

pub struct StdMethod {
    pub label: &'static str,
    pub insert: &'static str,
    pub detail: &'static str,
    pub doc: &'static str,
}

/// Methods available on `receiver_type`. Strips a leading `&`, trims `mut`,
/// and normalizes Copper aliases (`int`→`i64`, `string`→`String`, `str`→`&str`)
/// so callers can pass whatever type string the AST gave them.
pub fn methods_for(receiver_type: &str) -> &'static [StdMethod] {
    let normalized = normalize(receiver_type);
    match normalized.as_str() {
        "str" | "&str" | "String" | "string" => STRING_METHODS,
        "Vec" => VEC_METHODS,
        "Option" => OPTION_METHODS,
        "Result" => RESULT_METHODS,
        "HashMap" | "BTreeMap" => MAP_METHODS,
        "HashSet" | "BTreeSet" => SET_METHODS,
        "i8" | "i16" | "i32" | "i64" | "i128" | "isize" | "int" => INT_METHODS,
        "u8" | "u16" | "u32" | "u64" | "u128" | "usize" => UINT_METHODS,
        "f32" | "f64" | "float" => FLOAT_METHODS,
        "bool" => BOOL_METHODS,
        "char" => CHAR_METHODS,
        _ => &[],
    }
}

fn normalize(t: &str) -> String {
    let mut s = t.trim().to_string();
    while s.starts_with('&') || s.starts_with('*') {
        s = s[1..].trim().to_string();
    }
    if let Some(stripped) = s.strip_prefix("mut ") {
        s = stripped.trim().to_string();
    }
    if let Some(stripped) = s.strip_prefix("const ") {
        s = stripped.trim().to_string();
    }
    // Strip generic args: `Vec<i32>` → `Vec`, `Option<String>` → `Option`.
    if let Some(idx) = s.find('<') {
        s = s[..idx].trim().to_string();
    }
    s
}

/// Union of methods to show when the receiver type is unknown — better
/// than showing nothing on `name.<cursor>` when we can't infer the type.
/// Picks the most common methods from each table so the suggestions are
/// informative without being overwhelming.
pub fn union_methods() -> Vec<&'static StdMethod> {
    let mut out: Vec<&'static StdMethod> = Vec::new();
    for table in [STRING_METHODS, VEC_METHODS, OPTION_METHODS, RESULT_METHODS] {
        for m in table {
            if !out.iter().any(|x| x.label == m.label) {
                out.push(m);
            }
        }
    }
    out
}

pub fn to_completion_kind() -> CompletionItemKind {
    CompletionItemKind::METHOD
}

// ---- String / &str ----
const STRING_METHODS: &[StdMethod] = &[
    StdMethod {
        label: "len",
        insert: "len()",
        detail: "fn len(&self) -> usize",
        doc: "Length of the string in bytes (not chars).",
    },
    StdMethod {
        label: "is_empty",
        insert: "is_empty()",
        detail: "fn is_empty(&self) -> bool",
        doc: "`true` if the string has length 0.",
    },
    StdMethod {
        label: "to_uppercase",
        insert: "to_uppercase()",
        detail: "fn to_uppercase(&self) -> String",
        doc: "Allocate a new owned string with every char upper-cased.",
    },
    StdMethod {
        label: "to_lowercase",
        insert: "to_lowercase()",
        detail: "fn to_lowercase(&self) -> String",
        doc: "Allocate a new owned string with every char lower-cased.",
    },
    StdMethod {
        label: "trim",
        insert: "trim()",
        detail: "fn trim(&self) -> &str",
        doc: "Slice with leading and trailing whitespace removed.",
    },
    StdMethod {
        label: "trim_start",
        insert: "trim_start()",
        detail: "fn trim_start(&self) -> &str",
        doc: "Slice with leading whitespace removed.",
    },
    StdMethod {
        label: "trim_end",
        insert: "trim_end()",
        detail: "fn trim_end(&self) -> &str",
        doc: "Slice with trailing whitespace removed.",
    },
    StdMethod {
        label: "split",
        insert: "split(${1:pattern})",
        detail: "fn split<P>(&self, pat: P) -> impl Iterator<Item=&str>",
        doc: "Split on a pattern (char, &str, or closure). Lazy iterator.",
    },
    StdMethod {
        label: "split_whitespace",
        insert: "split_whitespace()",
        detail: "fn split_whitespace(&self) -> impl Iterator<Item=&str>",
        doc: "Split on Unicode whitespace, dropping empty matches.",
    },
    StdMethod {
        label: "lines",
        insert: "lines()",
        detail: "fn lines(&self) -> impl Iterator<Item=&str>",
        doc: "Iterate over the lines (split on `\\n` or `\\r\\n`).",
    },
    StdMethod {
        label: "contains",
        insert: "contains(${1:pattern})",
        detail: "fn contains<P>(&self, pat: P) -> bool",
        doc: "`true` if `pat` appears in the string.",
    },
    StdMethod {
        label: "starts_with",
        insert: "starts_with(${1:prefix})",
        detail: "fn starts_with<P>(&self, prefix: P) -> bool",
        doc: "`true` if the string begins with `prefix`.",
    },
    StdMethod {
        label: "ends_with",
        insert: "ends_with(${1:suffix})",
        detail: "fn ends_with<P>(&self, suffix: P) -> bool",
        doc: "`true` if the string ends with `suffix`.",
    },
    StdMethod {
        label: "replace",
        insert: "replace(${1:from}, ${2:to})",
        detail: "fn replace<P>(&self, from: P, to: &str) -> String",
        doc: "Replace every occurrence of `from` with `to`.",
    },
    StdMethod {
        label: "to_string",
        insert: "to_string()",
        detail: "fn to_string(&self) -> String",
        doc: "Allocate an owned `String` copy.",
    },
    StdMethod {
        label: "to_owned",
        insert: "to_owned()",
        detail: "fn to_owned(&self) -> String",
        doc: "Same as `to_string` for `&str`; clone for `String`.",
    },
    StdMethod {
        label: "parse",
        insert: "parse::<${1:T}>()",
        detail: "fn parse<T: FromStr>(&self) -> Result<T, T::Err>",
        doc: "Parse the string into a target type. Returns `Result`.",
    },
    StdMethod {
        label: "chars",
        insert: "chars()",
        detail: "fn chars(&self) -> impl Iterator<Item=char>",
        doc: "Iterate over Unicode scalar values.",
    },
    StdMethod {
        label: "bytes",
        insert: "bytes()",
        detail: "fn bytes(&self) -> impl Iterator<Item=u8>",
        doc: "Iterate over the raw UTF-8 bytes.",
    },
    StdMethod {
        label: "as_bytes",
        insert: "as_bytes()",
        detail: "fn as_bytes(&self) -> &[u8]",
        doc: "Borrow the underlying byte slice.",
    },
    StdMethod {
        label: "find",
        insert: "find(${1:pattern})",
        detail: "fn find<P>(&self, pat: P) -> Option<usize>",
        doc: "Byte index of the first match, or `None`.",
    },
    StdMethod {
        label: "push",
        insert: "push(${1:ch})",
        detail: "fn push(&mut self, ch: char)",
        doc: "Append a `char`. (Mutates — `String` only.)",
    },
    StdMethod {
        label: "push_str",
        insert: "push_str(${1:s})",
        detail: "fn push_str(&mut self, s: &str)",
        doc: "Append a `&str`. (Mutates — `String` only.)",
    },
    StdMethod {
        label: "clone",
        insert: "clone()",
        detail: "fn clone(&self) -> String",
        doc: "Deep copy of the string.",
    },
];

// ---- Vec<T> ----
const VEC_METHODS: &[StdMethod] = &[
    StdMethod {
        label: "len",
        insert: "len()",
        detail: "fn len(&self) -> usize",
        doc: "Number of elements.",
    },
    StdMethod {
        label: "is_empty",
        insert: "is_empty()",
        detail: "fn is_empty(&self) -> bool",
        doc: "`true` if `len() == 0`.",
    },
    StdMethod {
        label: "push",
        insert: "push(${1:item})",
        detail: "fn push(&mut self, value: T)",
        doc: "Append `value` to the back. Amortized O(1).",
    },
    StdMethod {
        label: "pop",
        insert: "pop()",
        detail: "fn pop(&mut self) -> Option<T>",
        doc: "Remove and return the last element, or `None` if empty.",
    },
    StdMethod {
        label: "insert",
        insert: "insert(${1:index}, ${2:value})",
        detail: "fn insert(&mut self, index: usize, element: T)",
        doc: "Insert `element` at `index`, shifting later elements right.",
    },
    StdMethod {
        label: "remove",
        insert: "remove(${1:index})",
        detail: "fn remove(&mut self, index: usize) -> T",
        doc: "Remove and return the element at `index`. Panics if out of bounds.",
    },
    StdMethod {
        label: "clear",
        insert: "clear()",
        detail: "fn clear(&mut self)",
        doc: "Drop every element.",
    },
    StdMethod {
        label: "iter",
        insert: "iter()",
        detail: "fn iter(&self) -> impl Iterator<Item=&T>",
        doc: "Borrowing iterator.",
    },
    StdMethod {
        label: "iter_mut",
        insert: "iter_mut()",
        detail: "fn iter_mut(&mut self) -> impl Iterator<Item=&mut T>",
        doc: "Mutably-borrowing iterator.",
    },
    StdMethod {
        label: "into_iter",
        insert: "into_iter()",
        detail: "fn into_iter(self) -> impl Iterator<Item=T>",
        doc: "Consuming iterator.",
    },
    StdMethod {
        label: "contains",
        insert: "contains(&${1:item})",
        detail: "fn contains(&self, x: &T) -> bool where T: PartialEq",
        doc: "`true` if any element equals `x`.",
    },
    StdMethod {
        label: "sort",
        insert: "sort()",
        detail: "fn sort(&mut self) where T: Ord",
        doc: "Sort in place, stable.",
    },
    StdMethod {
        label: "sort_by",
        insert: "sort_by(|a, b| ${1:a.cmp(b)})",
        detail: "fn sort_by<F>(&mut self, compare: F)",
        doc: "Sort in place using a custom comparator.",
    },
    StdMethod {
        label: "first",
        insert: "first()",
        detail: "fn first(&self) -> Option<&T>",
        doc: "First element, or `None` if empty.",
    },
    StdMethod {
        label: "last",
        insert: "last()",
        detail: "fn last(&self) -> Option<&T>",
        doc: "Last element, or `None` if empty.",
    },
    StdMethod {
        label: "get",
        insert: "get(${1:index})",
        detail: "fn get(&self, index: usize) -> Option<&T>",
        doc: "Bounds-checked element access.",
    },
    StdMethod {
        label: "clone",
        insert: "clone()",
        detail: "fn clone(&self) -> Vec<T> where T: Clone",
        doc: "Deep copy.",
    },
];

// ---- Option<T> ----
const OPTION_METHODS: &[StdMethod] = &[
    StdMethod {
        label: "is_some",
        insert: "is_some()",
        detail: "fn is_some(&self) -> bool",
        doc: "`true` if `self == Some(_)`.",
    },
    StdMethod {
        label: "is_none",
        insert: "is_none()",
        detail: "fn is_none(&self) -> bool",
        doc: "`true` if `self == None`.",
    },
    StdMethod {
        label: "unwrap",
        insert: "unwrap()",
        detail: "fn unwrap(self) -> T",
        doc: "Extract the value or panic if `None`.",
    },
    StdMethod {
        label: "unwrap_or",
        insert: "unwrap_or(${1:default})",
        detail: "fn unwrap_or(self, default: T) -> T",
        doc: "Extract the value or return `default` if `None`.",
    },
    StdMethod {
        label: "unwrap_or_else",
        insert: "unwrap_or_else(|| ${1:default})",
        detail: "fn unwrap_or_else<F>(self, f: F) -> T",
        doc: "Extract the value or compute a default lazily.",
    },
    StdMethod {
        label: "unwrap_or_default",
        insert: "unwrap_or_default()",
        detail: "fn unwrap_or_default(self) -> T where T: Default",
        doc: "Extract the value or `T::default()`.",
    },
    StdMethod {
        label: "map",
        insert: "map(|${1:x}| ${2:expr})",
        detail: "fn map<U, F>(self, f: F) -> Option<U>",
        doc: "Transform the inner value if `Some`.",
    },
    StdMethod {
        label: "and_then",
        insert: "and_then(|${1:x}| ${2:expr})",
        detail: "fn and_then<U, F>(self, f: F) -> Option<U>",
        doc: "Chain a function that itself returns `Option`.",
    },
    StdMethod {
        label: "or",
        insert: "or(${1:other})",
        detail: "fn or(self, optb: Option<T>) -> Option<T>",
        doc: "`self` if `Some`, else `optb`.",
    },
    StdMethod {
        label: "ok_or",
        insert: "ok_or(${1:err})",
        detail: "fn ok_or<E>(self, err: E) -> Result<T, E>",
        doc: "Convert into a `Result`, supplying an error for `None`.",
    },
    StdMethod {
        label: "as_ref",
        insert: "as_ref()",
        detail: "fn as_ref(&self) -> Option<&T>",
        doc: "Borrow the inner value.",
    },
    StdMethod {
        label: "take",
        insert: "take()",
        detail: "fn take(&mut self) -> Option<T>",
        doc: "Move the value out, leaving `None` behind.",
    },
    StdMethod {
        label: "clone",
        insert: "clone()",
        detail: "fn clone(&self) -> Option<T> where T: Clone",
        doc: "Deep copy.",
    },
];

// ---- Result<T, E> ----
const RESULT_METHODS: &[StdMethod] = &[
    StdMethod {
        label: "is_ok",
        insert: "is_ok()",
        detail: "fn is_ok(&self) -> bool",
        doc: "`true` if `self == Ok(_)`.",
    },
    StdMethod {
        label: "is_err",
        insert: "is_err()",
        detail: "fn is_err(&self) -> bool",
        doc: "`true` if `self == Err(_)`.",
    },
    StdMethod {
        label: "unwrap",
        insert: "unwrap()",
        detail: "fn unwrap(self) -> T",
        doc: "Extract the `Ok` value or panic with the `Err`.",
    },
    StdMethod {
        label: "unwrap_or",
        insert: "unwrap_or(${1:default})",
        detail: "fn unwrap_or(self, default: T) -> T",
        doc: "Extract the `Ok` value or return `default`.",
    },
    StdMethod {
        label: "unwrap_or_else",
        insert: "unwrap_or_else(|${1:e}| ${2:default})",
        detail: "fn unwrap_or_else<F>(self, f: F) -> T",
        doc: "Extract the `Ok` value or compute one from the error.",
    },
    StdMethod {
        label: "ok",
        insert: "ok()",
        detail: "fn ok(self) -> Option<T>",
        doc: "Convert to `Some(value)` on `Ok`, else `None`.",
    },
    StdMethod {
        label: "err",
        insert: "err()",
        detail: "fn err(self) -> Option<E>",
        doc: "Convert to `Some(error)` on `Err`, else `None`.",
    },
    StdMethod {
        label: "map",
        insert: "map(|${1:x}| ${2:expr})",
        detail: "fn map<U, F>(self, f: F) -> Result<U, E>",
        doc: "Transform the `Ok` value.",
    },
    StdMethod {
        label: "map_err",
        insert: "map_err(|${1:e}| ${2:expr})",
        detail: "fn map_err<F2, F>(self, f: F) -> Result<T, F2>",
        doc: "Transform the `Err` value.",
    },
    StdMethod {
        label: "and_then",
        insert: "and_then(|${1:x}| ${2:expr})",
        detail: "fn and_then<U, F>(self, f: F) -> Result<U, E>",
        doc: "Chain a function that itself returns `Result`.",
    },
    StdMethod {
        label: "expect",
        insert: "expect(${1:msg})",
        detail: "fn expect(self, msg: &str) -> T",
        doc: "Like `unwrap` but with a custom panic message.",
    },
];

// ---- HashMap / BTreeMap ----
const MAP_METHODS: &[StdMethod] = &[
    StdMethod {
        label: "len",
        insert: "len()",
        detail: "fn len(&self) -> usize",
        doc: "Number of entries.",
    },
    StdMethod {
        label: "is_empty",
        insert: "is_empty()",
        detail: "fn is_empty(&self) -> bool",
        doc: "`true` if no entries.",
    },
    StdMethod {
        label: "insert",
        insert: "insert(${1:key}, ${2:value})",
        detail: "fn insert(&mut self, k: K, v: V) -> Option<V>",
        doc: "Insert; returns the previous value if the key existed.",
    },
    StdMethod {
        label: "get",
        insert: "get(&${1:key})",
        detail: "fn get(&self, k: &K) -> Option<&V>",
        doc: "Lookup by reference.",
    },
    StdMethod {
        label: "remove",
        insert: "remove(&${1:key})",
        detail: "fn remove(&mut self, k: &K) -> Option<V>",
        doc: "Remove and return the value at `k`.",
    },
    StdMethod {
        label: "contains_key",
        insert: "contains_key(&${1:key})",
        detail: "fn contains_key(&self, k: &K) -> bool",
        doc: "`true` if `k` is a key.",
    },
    StdMethod {
        label: "keys",
        insert: "keys()",
        detail: "fn keys(&self) -> impl Iterator<Item=&K>",
        doc: "Iterator over keys.",
    },
    StdMethod {
        label: "values",
        insert: "values()",
        detail: "fn values(&self) -> impl Iterator<Item=&V>",
        doc: "Iterator over values.",
    },
    StdMethod {
        label: "iter",
        insert: "iter()",
        detail: "fn iter(&self) -> impl Iterator<Item=(&K, &V)>",
        doc: "Iterator over `(key, value)` pairs.",
    },
    StdMethod {
        label: "entry",
        insert: "entry(${1:key})",
        detail: "fn entry(&mut self, k: K) -> Entry<K, V>",
        doc: "Get an entry handle for in-place insert/update.",
    },
];

// ---- HashSet / BTreeSet ----
const SET_METHODS: &[StdMethod] = &[
    StdMethod {
        label: "len",
        insert: "len()",
        detail: "fn len(&self) -> usize",
        doc: "Number of elements.",
    },
    StdMethod {
        label: "is_empty",
        insert: "is_empty()",
        detail: "fn is_empty(&self) -> bool",
        doc: "`true` if empty.",
    },
    StdMethod {
        label: "insert",
        insert: "insert(${1:value})",
        detail: "fn insert(&mut self, value: T) -> bool",
        doc: "`true` if the value was newly inserted.",
    },
    StdMethod {
        label: "contains",
        insert: "contains(&${1:value})",
        detail: "fn contains(&self, value: &T) -> bool",
        doc: "Membership test.",
    },
    StdMethod {
        label: "remove",
        insert: "remove(&${1:value})",
        detail: "fn remove(&mut self, value: &T) -> bool",
        doc: "`true` if the value was present and removed.",
    },
    StdMethod {
        label: "iter",
        insert: "iter()",
        detail: "fn iter(&self) -> impl Iterator<Item=&T>",
        doc: "Iterator over elements.",
    },
];

// ---- Integer numeric types ----
const INT_METHODS: &[StdMethod] = &[
    StdMethod {
        label: "abs",
        insert: "abs()",
        detail: "fn abs(self) -> Self",
        doc: "Absolute value. Panics on i32::MIN etc.",
    },
    StdMethod {
        label: "pow",
        insert: "pow(${1:exp})",
        detail: "fn pow(self, exp: u32) -> Self",
        doc: "Integer exponentiation.",
    },
    StdMethod {
        label: "min",
        insert: "min(${1:other})",
        detail: "fn min(self, other: Self) -> Self",
        doc: "Smaller of `self` and `other`.",
    },
    StdMethod {
        label: "max",
        insert: "max(${1:other})",
        detail: "fn max(self, other: Self) -> Self",
        doc: "Larger of `self` and `other`.",
    },
    StdMethod {
        label: "checked_add",
        insert: "checked_add(${1:other})",
        detail: "fn checked_add(self, rhs: Self) -> Option<Self>",
        doc: "Addition that returns `None` on overflow.",
    },
    StdMethod {
        label: "saturating_add",
        insert: "saturating_add(${1:other})",
        detail: "fn saturating_add(self, rhs: Self) -> Self",
        doc: "Addition that clamps at the type bounds on overflow.",
    },
    StdMethod {
        label: "to_string",
        insert: "to_string()",
        detail: "fn to_string(&self) -> String",
        doc: "Decimal representation.",
    },
];

const UINT_METHODS: &[StdMethod] = &[
    StdMethod {
        label: "pow",
        insert: "pow(${1:exp})",
        detail: "fn pow(self, exp: u32) -> Self",
        doc: "Integer exponentiation.",
    },
    StdMethod {
        label: "min",
        insert: "min(${1:other})",
        detail: "fn min(self, other: Self) -> Self",
        doc: "Smaller of `self` and `other`.",
    },
    StdMethod {
        label: "max",
        insert: "max(${1:other})",
        detail: "fn max(self, other: Self) -> Self",
        doc: "Larger of `self` and `other`.",
    },
    StdMethod {
        label: "checked_sub",
        insert: "checked_sub(${1:other})",
        detail: "fn checked_sub(self, rhs: Self) -> Option<Self>",
        doc: "Subtraction that returns `None` on underflow.",
    },
    StdMethod {
        label: "leading_zeros",
        insert: "leading_zeros()",
        detail: "fn leading_zeros(self) -> u32",
        doc: "Count leading zero bits.",
    },
    StdMethod {
        label: "trailing_zeros",
        insert: "trailing_zeros()",
        detail: "fn trailing_zeros(self) -> u32",
        doc: "Count trailing zero bits.",
    },
    StdMethod {
        label: "to_string",
        insert: "to_string()",
        detail: "fn to_string(&self) -> String",
        doc: "Decimal representation.",
    },
];

// ---- Float types ----
const FLOAT_METHODS: &[StdMethod] = &[
    StdMethod {
        label: "abs",
        insert: "abs()",
        detail: "fn abs(self) -> Self",
        doc: "Absolute value.",
    },
    StdMethod {
        label: "sqrt",
        insert: "sqrt()",
        detail: "fn sqrt(self) -> Self",
        doc: "Square root.",
    },
    StdMethod {
        label: "powi",
        insert: "powi(${1:exp})",
        detail: "fn powi(self, n: i32) -> Self",
        doc: "Integer-exponent power.",
    },
    StdMethod {
        label: "powf",
        insert: "powf(${1:exp})",
        detail: "fn powf(self, n: Self) -> Self",
        doc: "Floating-exponent power.",
    },
    StdMethod {
        label: "floor",
        insert: "floor()",
        detail: "fn floor(self) -> Self",
        doc: "Largest integer ≤ self.",
    },
    StdMethod {
        label: "ceil",
        insert: "ceil()",
        detail: "fn ceil(self) -> Self",
        doc: "Smallest integer ≥ self.",
    },
    StdMethod {
        label: "round",
        insert: "round()",
        detail: "fn round(self) -> Self",
        doc: "Round half-away-from-zero.",
    },
    StdMethod {
        label: "is_nan",
        insert: "is_nan()",
        detail: "fn is_nan(self) -> bool",
        doc: "`true` if NaN.",
    },
    StdMethod {
        label: "to_string",
        insert: "to_string()",
        detail: "fn to_string(&self) -> String",
        doc: "Default decimal representation.",
    },
];

// ---- Bool ----
const BOOL_METHODS: &[StdMethod] = &[
    StdMethod {
        label: "then",
        insert: "then(|| ${1:value})",
        detail: "fn then<T, F>(self, f: F) -> Option<T>",
        doc: "`Some(f())` if `self`, else `None`.",
    },
    StdMethod {
        label: "then_some",
        insert: "then_some(${1:value})",
        detail: "fn then_some<T>(self, t: T) -> Option<T>",
        doc: "`Some(t)` if `self`, else `None`.",
    },
    StdMethod {
        label: "to_string",
        insert: "to_string()",
        detail: "fn to_string(&self) -> String",
        doc: "`\"true\"` or `\"false\"`.",
    },
];

// ---- Char ----
const CHAR_METHODS: &[StdMethod] = &[
    StdMethod {
        label: "is_alphabetic",
        insert: "is_alphabetic()",
        detail: "fn is_alphabetic(self) -> bool",
        doc: "Unicode-aware alphabetic check.",
    },
    StdMethod {
        label: "is_alphanumeric",
        insert: "is_alphanumeric()",
        detail: "fn is_alphanumeric(self) -> bool",
        doc: "Alphabetic or numeric.",
    },
    StdMethod {
        label: "is_ascii",
        insert: "is_ascii()",
        detail: "fn is_ascii(self) -> bool",
        doc: "`true` if the char is in `0..=127`.",
    },
    StdMethod {
        label: "is_digit",
        insert: "is_digit(${1:10})",
        detail: "fn is_digit(self, radix: u32) -> bool",
        doc: "Digit in the given radix.",
    },
    StdMethod {
        label: "to_uppercase",
        insert: "to_uppercase()",
        detail: "fn to_uppercase(self) -> impl Iterator<Item=char>",
        doc: "Uppercase mapping (may yield more than one char).",
    },
    StdMethod {
        label: "to_lowercase",
        insert: "to_lowercase()",
        detail: "fn to_lowercase(self) -> impl Iterator<Item=char>",
        doc: "Lowercase mapping (may yield more than one char).",
    },
    StdMethod {
        label: "to_string",
        insert: "to_string()",
        detail: "fn to_string(&self) -> String",
        doc: "One-char `String`.",
    },
];
