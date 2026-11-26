# Changelog

## 0.1.0

- Initial `@nuclescript/std` package release, written in NucleScript.
- Added `clear`, which deletes every file a pool holds.
- Added `each_file`, which runs a closure on each file a pool holds.
- Added `prune`, `each_file` with a `confirm physical_key` ceiling on the
  closure.
