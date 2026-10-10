//! Caller JSON-LD is bounded in JSON NESTING before json-ld sees it, and the algorithms run on
//! a thread sized for the bound (ledger #1042).
//!
//! json-ld 0.21's algorithms recurse once per nested JSON object AND per nested array:
//! expansion (`json-ld-expansion`'s `expand_element` → `expand_node` → `expand_element`, and
//! `expand_array` for every array), and compaction and flattening on top of it. A stack
//! overflow aborts the WHOLE host, on any thread. Measured by `tests/jsonld_depth_measure.rs`
//! (json-ld 0.21.4, aarch64-apple-darwin, a 2 MiB thread, the size of a tokio worker's), the
//! largest `n` that survived, with the JSON depth (`{` and `[` alike) of that document:
//!
//! | shape, at expand · flatten · compact | debug | release |
//! | --- | --- | --- |
//! | node objects as values, nested blank nodes (one JSON level a level) | 12 | 122–123 |
//! | node objects in arrays, nested `@graph`s (two levels a level) | 8 (17 deep) | 87–88 (~175) |
//! | `@reverse` objects (two levels a level) | 12 (25 deep) | 122–123 (~245) |
//! | arrays of arrays: as a value, at the top, inside `@list` | 28–29 | 305–307 |
//! | `@list` objects in `@list` objects (two levels a level) | 32 (65 deep) | 305–307 (~613) |
//! | scoped `@context`s in term definitions (two levels a level) | 48–50 (~100 deep) | 314–318 (~632) |
//! | arrays in `@context`; a `@json` literal | ~1,340–1,490 | ~5,120–5,200 |
//! | unmapped keys (dropped by expansion) | ~6,150 | ≥ 16,384 |
//!
//! json-syntax's parser itself is iterative; what the last row measures is dropping the parsed
//! value, which recurses too (~6,180 levels in debug).
//!
//! # Memory: linear in the depth, per level of `@list`
//!
//! oxjsonld's quadratic memory (ledger #1038) is NOT json-ld's: plain node objects add no copy
//! per level (an innermost node of 20,000 values, ~0.5 MB: peak 17 MiB at one level, 35 MiB at
//! 64, 84 MiB at 256, release). But the expander CLONES a `@list` object's value before it
//! expands it (`json-ld-expansion` 0.21 `element.rs`, `list_entry = Some(value.clone())`), and
//! every level's clone stays live under the recursion, so `@list` objects nested around that
//! same 0.5 MB cost ~2.6 MiB a level: 98 MiB at 32 levels (JSON depth 65), 682 MiB at 256.
//! Memory is the depth times the parsed document, which the bound caps at 64 times it. The
//! printed output grows the same way (pretty-printing indents every line by its depth), and
//! the bound caps that too.
//!
//! Hence two layers, as ikigai-rdf's `depth` does for oxjsonld:
//!
//! 1. [`check_json_nesting`] refuses JSON nested deeper than [`MAX_JSON_NESTING`] (64), as a
//!    typed `InvalidArgument` naming the argument (`content`, or `compact`'s `context`), before
//!    json-ld sees a byte.
//! 2. The parse, the algorithm and the print run on a thread of [`JSON_LD_STACK`] (32 MiB), so a
//!    debug build holds the bound too: at JSON depth 64 it needs ~9.1 MiB (node objects) and a
//!    release build ~1.1 MiB, and a debug and a release host admit and answer the same
//!    documents. ⚠ On wasm there is no thread: the work runs inline and the bound alone applies.
//!    An optimized wasm32 build needs 256–384 KiB at JSON depth 64 (node objects), of the 1 MiB
//!    stack rustc gives a wasm32 binary by default; a DEBUG wasm build traps at the bound
//!    (measured on wasm32-wasip1 under node's WASI, `-zstack-size` varied, 2026-10-10). The
//!    `module` build is a release build, so the bound holds there with ~3× to spare.

use ikigai_core::{Error, Result};

// COPY: `MAX_JSON_NESTING` and `check_json_nesting` are ikigai-rdf's (`src/depth.rs`,
// ikigai-rs/ikigai-linkeddata PR 38, ledger #1038), copied verbatim but for the refusal's
// wording, which names json-ld's recursion rather than oxjsonld's. ikigai-rdf exports both, but
// not from a published release yet (0.2.0 predates them), and depending on it would pull
// oxrdfio and quick-xml into this module's wasm, which exists to keep heavy trees out. The fold
// is ledger #976's shared limits crate; until it lands, change both copies together.

/// How deep caller JSON-LD may nest JSON objects and arrays, counted together, outside strings:
/// **64**, the same number as ikigai-rdf's, so one bound on caller nesting holds across the
/// doors.
///
/// Real JSON-LD is shallow: the deepest of the 2,146 documents in the W3C JSON-LD 1.1 API test
/// suite (expand, compact, flatten, toRdf, fromRdf) nests 10, and every JSON-LD file in the
/// ikigai ecosystem (the vocabulary's context among them) 3 (measured 2026-10-10, ledger #1038).
/// Arrays count because json-ld recurses on them as well as on objects.
pub const MAX_JSON_NESTING: usize = 64;

/// The stack the JSON-LD work is given on a thread of its own (native only): 32 MiB.
///
/// A debug build spends ~145 KiB of stack per nested node object and a release one ~17 KiB, so
/// a document at [`MAX_JSON_NESTING`] needs ~9.1 MiB in debug, more than four tokio workers'
/// worth; 32 MiB is three and a half times that. Reserved address space, touched only as deep as
/// a document recurses.
pub const JSON_LD_STACK: usize = 32 << 20;

/// Refuse caller JSON (here, JSON-LD) that nests objects and arrays deeper than
/// [`MAX_JSON_NESTING`], without parsing it.
///
/// One pass, no recursion, and it reads strings as JSON does: a string opens at a `"` outside
/// one and closes at the next `"` not escaped by `\`, so a bracket inside a string is text.
/// It counts the whole input, past the point where the parser would stop at an error, which is
/// the safe direction (it can over-count, never hide a level the parser reads).
///
/// ```
/// use ikigai_jsonld::{check_json_nesting, MAX_JSON_NESTING};
///
/// let nodes = |n: usize| {
///     format!("{}{{}}{}", "{\"urn:ex:p\":".repeat(n - 1), "}".repeat(n - 1))
/// };
/// assert!(check_json_nesting(nodes(MAX_JSON_NESTING).as_bytes(), "content").is_ok());
/// let refusal = check_json_nesting(nodes(MAX_JSON_NESTING + 1).as_bytes(), "content")
///     .unwrap_err()
///     .to_string();
/// assert!(refusal.starts_with("invalid argument `content`"), "{refusal}");
/// assert!(refusal.contains("deeper than 64 (MAX_JSON_NESTING)"), "{refusal}");
/// // Brackets in a string are not nesting.
/// let quoted = format!("{{\"@id\":\"urn:ex:s\",\"urn:ex:p\":\"{}\"}}", "[{".repeat(100));
/// assert!(check_json_nesting(quoted.as_bytes(), "content").is_ok());
/// ```
pub fn check_json_nesting(bytes: &[u8], arg: &str) -> Result<()> {
    let mut depth = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                // A string: ends at the first `"` a `\` does not escape. The loop leaves `i` on
                // that quote, and the step below moves past it.
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    i += if bytes[i] == b'\\' { 2 } else { 1 };
                }
            }
            b'{' | b'[' => {
                depth += 1;
                if depth > MAX_JSON_NESTING {
                    return Err(Error::InvalidArgument {
                        name: arg.to_string(),
                        detail: format!(
                            "this JSON-LD nests JSON objects and arrays deeper than \
                             {MAX_JSON_NESTING} (MAX_JSON_NESTING): the JSON-LD algorithms \
                             recurse once per nested object and array, where a stack overflow \
                             aborts the whole host, and copy a `@list`'s content once per \
                             level. Real JSON-LD nests about 10 deep at most; flatten this one"
                        ),
                    });
                }
            }
            // A closer with nothing open is a syntax error the parser stops at.
            b'}' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
        i += 1;
    }
    Ok(())
}

/// Run `work`, a JSON-LD parse and what consumes it, on a thread of [`JSON_LD_STACK`], and wait
/// for it. A panic there is resumed here, so the caller sees what it would have inline; a thread
/// that cannot be spawned is a transient [`Error::Unavailable`], never a fallback to running
/// inline, which is the overflow this exists to prevent. On wasm there are no threads, and
/// `work` runs inline.
pub(crate) fn on_json_ld_stack<T, F>(work: F) -> Result<T>
where
    T: Send,
    F: FnOnce() -> Result<T> + Send,
{
    #[cfg(not(target_family = "wasm"))]
    {
        std::thread::scope(|scope| {
            let handle = std::thread::Builder::new()
                .name("ikigai-jsonld".to_string())
                .stack_size(JSON_LD_STACK)
                .spawn_scoped(scope, work)
                .map_err(|e| {
                    Error::Unavailable(format!(
                        "could not start a {} MiB thread to process this JSON-LD on: {e}",
                        JSON_LD_STACK >> 20
                    ))
                })?;
            match handle.join() {
                Ok(result) => result,
                Err(panic) => std::panic::resume_unwind(panic),
            }
        })
    }
    #[cfg(target_family = "wasm")]
    {
        work()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bound_is_the_shared_number() {
        assert_eq!(MAX_JSON_NESTING, 64);
    }

    #[test]
    fn arrays_count_as_levels_and_siblings_do_not() {
        let arrays = |n: usize| format!("{}1{}", "[".repeat(n), "]".repeat(n));
        check_json_nesting(arrays(64).as_bytes(), "content").unwrap();
        assert!(check_json_nesting(arrays(65).as_bytes(), "content").is_err());
        let siblings = format!("[{}{{}}]", "{\"a\":[1]},".repeat(5_000));
        check_json_nesting(siblings.as_bytes(), "content").unwrap();
    }

    #[test]
    fn an_escaped_quote_does_not_end_a_string() {
        // `\"` keeps the string open, so the brackets after it are still text; `\\` before the
        // closing quote is an escaped backslash, and the brackets after THAT are nesting.
        let text = format!("[\"a\\\"{}\"]", "[".repeat(100));
        check_json_nesting(text.as_bytes(), "content").unwrap();
        let text = format!("[\"a\\\\\"{}1{}]", "[".repeat(100), "]".repeat(100));
        assert!(check_json_nesting(text.as_bytes(), "content").is_err());
    }

    #[test]
    fn the_work_runs_on_the_sized_thread_and_a_panic_comes_back() {
        let name = on_json_ld_stack(|| Ok(std::thread::current().name().map(str::to_string)));
        assert_eq!(name.unwrap().as_deref(), Some("ikigai-jsonld"));
        let caught = std::panic::catch_unwind(|| on_json_ld_stack::<(), _>(|| panic!("inside")));
        assert!(caught.is_err());
    }
}
