# Accessibility

What `ikigai-jsonld` emits that people read, the rules that output follows, how it is tested, and
how to report a barrier. This crate writes JSON documents and the text of its errors; it writes no
HTML, SVG or terminal output of its own. It is not a conformance claim. Adapted from the
[ACCESSIBILITY.md template](https://github.com/mgifford/ACCESSIBILITY.md) (MIT).

| Field | Value |
|---|---|
| Target | WCAG 2.2 Level AA, for any page that presents this output; this repository writes text only |
| Status | Evaluation in progress. No conformance claim. |
| Owner | the maintainers of this repository |
| Report a barrier | [a GitHub issue](https://github.com/ikigai-rs/ikigai-jsonld/issues/new?labels=accessibility) with the `accessibility` label; anything security-sensitive through [private vulnerability reporting](https://github.com/ikigai-rs/ikigai-jsonld/security/advisories/new) |
| Last reviewed | 2026-10-31 |

## What it emits that people read

- **JSON documents**: the answers of `urn:jsonld:expand`, `flatten`, `compact` and `prune`
  (`src/lib.rs`, `src/prune.rs`), pretty-printed with one member per indented line, and
  `prune`'s `face=report`, a JSON summary of what was removed (`dropped`, `properties`,
  `reverseProperties`, `types`, `datatypes`, `emptyNodes`). They are data for programs that
  people also read.
- **Error and refusal messages**: the sentences in `Error::Endpoint(format!(…))`,
  `Error::InvalidArgument` and `Error::Unavailable` in `src/lib.rs`, `src/prune.rs` and
  `src/depth.rs`, which a host shows to whoever made the request.

Not this repository's: the page, error page or terminal a host puts any of this in.

## The rules that output follows

Followed by reading the source:

- A refusal of a bad argument is an `InvalidArgument` that names the argument (`content`,
  `context`, `face`) and says why in a sentence; most also say what to do: an unknown `face`
  lists the two that work, a context with `@vocab` says an egress context must name each term
  it allows, and a document nested too deep says the bound (64, `MAX_JSON_NESTING`) and asks for
  it to be flattened.
- Nothing is written in color; every answer is plain text.

## How it is tested

- `tests/prune.rs`: `a_context_with_vocab_is_refused` and `a_scoped_context_is_refused` check
  that the refusal names `context` and says why; `an_unknown_face_is_refused` checks that it
  names `face`.
- `tests/json_depth.rs` (`deep_documents_are_refused_at_every_door_and_abort_nothing`,
  `a_deep_context_is_refused_naming_context_inline_and_by_reference`) checks that a document
  nested too deep is refused naming the argument, with the words `deeper than 64
  (MAX_JSON_NESTING)`.
- **No automated accessibility checker, browser or assistive technology has been used on this
  output.**

## Known limitations

Found by reading the source on 2026-10-31, not by testing:

- Several errors name no argument and say nothing about what to do, only the stage and the
  underlying library's message: for example `JSON-LD parse error: …`, `JSON-LD expand failed:
  …`, and the `… did not complete synchronously` errors.

## Contributing

Read this file before changing any output listed above: an error message or the shape of an
answer. Keep an error a sentence that names the argument and says what to do. Say in the pull
request what was tested, with what, and what was not.

## Reporting a barrier

[Open an issue](https://github.com/ikigai-rs/ikigai-jsonld/issues/new?labels=accessibility) with
the `accessibility` label (the link sets it when GitHub lets you add labels; otherwise a maintainer
adds it): what you were doing, what happened, and your terminal or browser and any assistive
technology, if you are comfortable sharing them. Anything security-sensitive goes through
[private vulnerability reporting](https://github.com/ikigai-rs/ikigai-jsonld/security/advisories/new).
**We acknowledge a report within 7 days**, and promise nothing further.

## Change history

| Date | Change |
|---|---|
| 2026-10-31 | First version. |
