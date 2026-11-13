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
| Last reviewed | 2026-11-13 (UTC) |

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

Source review of main (2026-11-13); the results and limits of verification are below:

- Selected argument refusals use `InvalidArgument` to name the argument (`content`,
  `context`, `face`) and explain why; examples also say what to do: an unknown `face`
  lists the two that work, a context with `@vocab` says an egress context must name each term
  it allows, and a document nested too deep says the bound (64, `MAX_JSON_NESTING`) and asks for
  it to be flattened.
- A context dependency chain past 64 (`MAX_CONTEXT_DEPENDENCY_DEPTH`) is an
  `InvalidArgument` naming `content` or `context` and asking for direct IRI mappings
  to shorten the dependencies.
- Nothing is written in color; every answer is plain text.

## How it is tested

Every named check has a result: Pass, Fail or Blocked, with evidence. A Pass covers only
that check, not overall accessibility or WCAG conformance. The automated checks below ran on
2026-11-13 (UTC), on main commit
[`fe6967068afa139076f6275f54f5793e8cad07d8`](https://github.com/ikigai-rs/ikigai-jsonld/commit/fe6967068afa139076f6275f54f5793e8cad07d8),
using Cargo 1.99.0 (5f94df478 2026-08-27) and rustc 1.99.0 (b940084d7 2026-09-28).
The command was `cargo test --test prune --test json_depth --test context_depth`.
CI's Rust test gate repeats these integration tests; it does not evaluate a host's presentation
or run assistive technology. These tests assert selected error fields and text, not whether
people can understand every error or read the resulting JSON comfortably.

| Check | Result | Evidence |
|---|---|---|
| Unsafe-context and unknown-face refusals | Pass | `tests/prune.rs`: `a_context_with_vocab_is_refused` and `a_scoped_context_is_refused` check that the refusal names `context` and mentions `@vocab` or scoped contexts; `an_unknown_face_is_refused` checks the argument name `face`, not the recovery wording. Run with the command and tool versions above. |
| JSON nesting-depth refusals | Pass | `tests/json_depth.rs`: `deep_documents_are_refused_at_every_door_and_abort_nothing` and `a_deep_context_is_refused_naming_context_inline_and_by_reference` check the argument name and `deeper than 64 (MAX_JSON_NESTING)`, including inline and referenced contexts. Run with the command and tool versions above; no human readability check. |
| Context dependency-depth refusals | Pass | `tests/context_depth.rs`: `long_flat_chains_are_refused_at_every_door_without_aborting` checks the argument name and `deeper than 64 (MAX_CONTEXT_DEPENDENCY_DEPTH)`, including inline and referenced target contexts. Run with the command and tool versions above; does not check the recovery instruction's usability. |
| Every error names the argument and explains recovery | Fail | Source review on 2026-11-13 at the main commit above: `parse_doc` and `expand_doc` in `src/lib.rs` emit `JSON-LD parse error: …`, `JSON-LD expand failed: …` and synchronization errors without an argument name or recovery instruction. This confirms the known limitation below; it is not a complete error audit. |
| JSON formatting and absence of color in a host's presentation | Blocked | Source uses pretty-printing and plain error text, but no host rendering session was available for this review. Tests above do not establish visual readability or that a host preserves meaning without color. |
| Rendered documentation accessibility | Blocked | Documentation is Markdown read on GitHub or in an editor; no rendered documentation evaluation was run for this update. |
| Browser presentation, zoom, reflow and contrast | Blocked | This crate emits no page; no consuming host page and browser evaluation session were available. These properties depend on the host. |
| Manual keyboard | Blocked | No person performed a keyboard pass on a host presenting this output; no interactive host session was available for this update. |
| Automated accessibility checker (axe or equivalent) | Blocked | No automated accessibility checker has been used on this output; no host page or browser checker was configured for this update. Rust tests are not an accessibility checker. |
| Screen reader | Blocked | No screen reader has been used on this output; no screen reader and host session were available for this update. |
| Screen magnifier | Blocked | No screen magnifier has been used on this output; no magnifier and host session were available for this update. |
| Speech input | Blocked | No speech-input software has been used on this output; no speech-input and host session were available for this update. |
| Switch access | Blocked | No switch-access software has been used on this output; no switch-access and host session were available for this update. |
| Braille display | Blocked | No braille display has been used on this output; no braille display and host session were available for this update. |

## Known limitations

- Source review on 2026-11-13 reconfirmed the limitation first recorded on 2026-10-31:
  several errors name no argument and say nothing about what to do, only the stage and the
  underlying library's message. Examples are `JSON-LD parse error: …`, `JSON-LD expand failed:
  …`, and the `… did not complete synchronously` errors. The Fail row above records this
  against the error-writing rule; this documentation update does not change those messages.
- The Blocked checks above leave human readability, host presentation and assistive-technology
  use unevaluated. Plain text alone does not establish accessibility.

## Contributing

Read this file before changing any output listed above: an error message or the shape of an
answer. Keep an error a sentence that names the argument and says what to do. In the pull
request's accessibility section, give each check a row in a Check | Result | Evidence table:
Pass, Fail or Blocked, with the test or command, tool and version used, or why it could not run.
Keep unrun checks as Blocked, including assistive technology nobody used. Historical checks
that CI does not repeat are Blocked with their date, not Pass.

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
| 2026-11-11 | Added the context dependency-depth refusal and its child-process checks. |
| 2026-11-13 | Recorded Pass, Fail and Blocked results, current main test evidence and explicit assistive-technology gaps. |
