# Changelog

## 0.5.3

- Add an opt-in offline loader with SHA-256-pinned VC Data Model 2.0, Open Badges
  3.0.3, and Multikey contexts. The `urn:jsonld:*` endpoints retain their existing
  loader. No persisted state changes and nothing to migrate.

## 0.5.2

- Invalid `base` values on all four JSON-LD operations and invalid `face` values on `prune` that previously selected a default now return an error naming the argument. Omitted arguments still use the same defaults. No persisted state changes.
