# xml-fragment

Opaque, re-emittable XML subtrees for [serde](https://serde.rs) documents parsed with
[quick-xml](https://github.com/tafia/quick-xml).

`XmlFragment` captures an arbitrary XML element as a structural value — maps, sequences and
scalars, with attributes as `@name` keys — and writes it back. Use it when a document contains an
element whose concrete type you don't know (a plugin payload, a versioned schema, an "any" field)
and you want to preserve it instead of dropping it or failing the parse.

## quick-xml is a hard dependency

This crate is built for quick-xml and always depends on it:

- `into_typed` / `from_typed` call `quick_xml::de::from_str` / `quick_xml::se::to_string`.
- The value model follows quick-xml's serde conventions: attributes are `@name` keys, text-only
  elements arrive as `$text`, and namespace prefixes are stripped (`xsi:type` becomes `@type`,
  normalized back to `@xsi:type` on capture).
- The `Serialize` / `Deserialize` impls are ordinary serde impls, so they *can* be used with any
  serde `Serializer`/`Deserializer`, but another XML backend would need to produce the same
  attribute/text conventions.

It is not a parser abstraction layer.

## Example

```rust
use serde::{Deserialize, Serialize};
use xml_fragment::XmlFragment;

#[derive(Deserialize, Serialize)]
struct Holder {
    #[serde(rename = "Data")]
    data: XmlFragment,
}

let xml = r#"<Holder><Data xsi:type="SomeType"><Child A="1">text</Child></Data></Holder>"#;

let holder: Holder = quick_xml::de::from_str(xml).unwrap();
let out = quick_xml::se::to_string(&holder).unwrap();

assert!(out.contains(r#"xsi:type="SomeType""#));
assert!(out.contains(r#"A="1""#));
```

## What is preserved

- element names, nesting and order
- attributes (`@name` keys), including namespace declarations such as `xmlns:ns`
- repeated child elements
- text content, including CDATA content (the CDATA syntax itself is not preserved)

## What is not preserved

- whitespace and attribute order
- comments and processing instructions — quick-xml's event API reads and writes them, but serde
  has no comment node and quick-xml's serde layer skips them, so a serde-value crate cannot see
  them; preserving them would require an event-based capture mode instead
- `<X/>` vs `<X></X>`
- namespace prefixes on element and attribute names (quick-xml strips them; `xsi:type` is
  normalized back from `@type` specially)

## Why a separate crate?

quick-xml's serde module provides attribute and text-content helpers but no opaque subtree
capture, and the other serde XML crates do not provide one either. DOM crates such as `roxmltree`
and `xmltree` are not serde values. This crate fills that gap for quick-xml-based parsers; it is
deliberately not a parser abstraction layer.

## Features

None. The crate has no optional dependencies.

## Releasing

Releases are automated with [release-plz](https://release-plz.dev):

- Commits to `main` follow [Conventional Commits](https://www.conventionalcommits.org)
  (`fix:`, `feat:`, `feat!:` or `BREAKING CHANGE:`).
- On every push, release-plz opens or updates a release PR that bumps `Cargo.toml` and updates
  `CHANGELOG.md`. While the crate is pre-1.0, features bump the patch version and breaking changes
  bump the minor version, per Cargo's semver rules.
- Merging the release PR creates the `vX.Y.Z` tag, the GitHub release and runs `cargo publish`.

Required repository secrets:

- `RELEASE_PLZ_TOKEN` — a PAT with `contents: write` and `pull-requests: write`. Using a PAT
  instead of the default `GITHUB_TOKEN` lets CI run on the release PR and lets release-plz push
  the tag.
- `CARGO_REGISTRY_TOKEN` — crates.io API token.

Configuration lives in `release-plz.toml`. `semver_check` is enabled: release-plz runs `cargo-semver-checks` (installed by the action) and bumps the version when it detects an API break.

## License

MIT
