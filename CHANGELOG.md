# Changelog

## 0.5.2

- Invalid `base` values on all four JSON-LD operations and invalid `face` values on `prune` that previously selected a default now return an error naming the argument. Omitted arguments still use the same defaults. No persisted state changes.
