//! Opaque, re-emittable XML subtrees for serde.
//!
//! [`XmlFragment`] captures an arbitrary XML element as a structural value (maps, sequences and
//! scalars, with attributes as `@name` keys) and writes it back unchanged. It is meant as a
//! fallback for "any" elements whose concrete type is unknown or not worth modelling:
//!
//! ```
//! use serde::{Deserialize, Serialize};
//! use xml_fragment::XmlFragment;
//!
//! #[derive(Deserialize, Serialize)]
//! struct Holder {
//!     #[serde(rename = "Data")]
//!     data: XmlFragment,
//! }
//!
//! let xml = r#"<Holder><Data xsi:type="SomeType"><Child A="1"/></Data></Holder>"#;
//! let holder: Holder = quick_xml::de::from_str(xml).unwrap();
//! let out = quick_xml::se::to_string(&holder).unwrap();
//! assert!(out.contains(r#"xsi:type="SomeType""#));
//! ```
//!
//! The fragment preserves element names, attributes, nesting, repeated elements and text
//! content (including CDATA content, without the CDATA syntax). It does not preserve whitespace,
//! attribute order, `<X/>` vs `<X></X>`, or namespace prefixes on element and attribute names
//! (quick-xml strips them; `xsi:type` is normalized back from `@type`). Comments and processing
//! instructions are dropped: quick-xml's event API supports them, but serde has no comment node
//! and quick-xml's serde layer skips them, so a serde-value crate cannot capture them.
//!
//! quick-xml is a hard dependency: [`XmlFragment::into_typed`] and [`XmlFragment::from_typed`]
//! call its serde (de)serializer, and the value model follows its conventions (`@name`
//! attributes, `$text`, stripped namespace prefixes). The `Serialize`/`Deserialize` impls are
//! ordinary serde impls, but another XML backend would need to match those conventions. This is
//! not a parser abstraction layer.

use std::fmt;

use serde::de::{DeserializeOwned, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::ser::{SerializeMap, SerializeSeq, Serializer};
use serde::{Deserialize, Serialize};

/// A captured XML subtree: either an element's content (map/sequence/scalar) or nothing.
#[derive(Debug, Clone, PartialEq)]
pub enum XmlFragment {
    Map(Vec<(String, XmlFragment)>),
    Seq(Vec<XmlFragment>),
    Scalar(String),
}

impl Default for XmlFragment {
    fn default() -> Self {
        Self::Scalar(String::new())
    }
}

impl XmlFragment {
    /// Look up a map entry by key (e.g. `"@xsi:type"` or a child element name).
    pub fn get(&self, key: &str) -> Option<&XmlFragment> {
        match self {
            Self::Map(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// Scalar text, if this fragment is a scalar.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Scalar(value) => Some(value),
            _ => None,
        }
    }

    /// The concrete object-builder type name declared in `xsi:type`.
    ///
    /// quick-xml strips namespace prefixes, so the key arrives as `@type`;
    /// [`Self`] normalizes it back to `@xsi:type` when capturing.
    pub fn type_attribute(&self) -> Option<&str> {
        self.get("@xsi:type").and_then(Self::as_str)
    }

    /// Deserialize the captured element into a typed representation.
    ///
    /// The fragment is rendered back to XML and parsed with quick-xml, so the result follows
    /// exactly the same serde rules as parsing the original document.
    pub fn into_typed<T: DeserializeOwned>(&self) -> Result<T, String> {
        let xml = self.to_xml_string();
        quick_xml::de::from_str::<T>(&xml).map_err(|e| e.to_string())
    }

    /// Capture a known value as a fragment and declare its `xsi:type`.
    pub fn from_typed<T: Serialize>(value: &T, type_name: &str) -> Result<Self, String> {
        let xml = quick_xml::se::to_string(value).map_err(|e| e.to_string())?;
        let fragment = quick_xml::de::from_str::<XmlFragment>(&xml).map_err(|e| e.to_string())?;

        let mut entries = match fragment {
            Self::Map(entries) => entries,
            Self::Scalar(text) => vec![("$text".to_string(), Self::Scalar(text))],
            Self::Seq(_) => return Err("cannot embed a sequence as an object builder".to_string()),
        };

        entries.retain(|(key, _)| key != "@xsi:type" && key != "@type");
        entries.insert(
            0,
            (
                "@xsi:type".to_string(),
                Self::Scalar(type_name.to_string()),
            ),
        );

        Ok(Self::Map(entries))
    }

    /// Render this fragment as a standalone XML element named `Value`.
    fn to_xml_string(&self) -> String {
        let mut out = String::new();
        self.render("Value", &mut out);
        out
    }

    fn render(&self, name: &str, out: &mut String) {
        match self {
            Self::Scalar(text) => {
                out.push('<');
                out.push_str(name);
                out.push('>');
                out.push_str(&escape_text(text));
                out.push_str("</");
                out.push_str(name);
                out.push('>');
            }
            Self::Seq(items) => {
                for item in items {
                    item.render(name, out);
                }
            }
            Self::Map(entries) => {
                let mut attributes = String::new();
                let mut body = String::new();

                for (key, value) in entries {
                    if let Some(attribute) = key.strip_prefix('@') {
                        let rendered = value.as_str().unwrap_or_default();
                        attributes.push(' ');
                        attributes.push_str(attribute);
                        attributes.push_str("=\"");
                        attributes.push_str(&escape_attribute(rendered));
                        attributes.push('"');
                    } else if key == "$text" {
                        // Keep text in document order relative to child elements.
                        body.push_str(&escape_text(value.as_str().unwrap_or_default()));
                    } else {
                        value.render(key, &mut body);
                    }
                }

                out.push('<');
                out.push_str(name);
                out.push_str(&attributes);
                if body.is_empty() {
                    out.push_str("/>");
                } else {
                    out.push('>');
                    out.push_str(&body);
                    out.push_str("</");
                    out.push_str(name);
                    out.push('>');
                }
            }
        }
    }
}

fn escape_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_attribute(value: &str) -> String {
    escape_text(value)
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

impl<'de> Deserialize<'de> for XmlFragment {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct FragmentVisitor;

        impl<'de> Visitor<'de> for FragmentVisitor {
            type Value = XmlFragment;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("an XML element")
            }

            fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
                Ok(XmlFragment::Scalar(value.to_string()))
            }

            fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
                Ok(XmlFragment::Scalar(value.to_string()))
            }

            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
                Ok(XmlFragment::Scalar(value.to_string()))
            }

            fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E> {
                Ok(XmlFragment::Scalar(value.to_string()))
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
                Ok(XmlFragment::Scalar(value.to_string()))
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
                Ok(XmlFragment::Scalar(value))
            }

            fn visit_none<E>(self) -> Result<Self::Value, E> {
                Ok(XmlFragment::Scalar(String::new()))
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(XmlFragment::Scalar(String::new()))
            }

            fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
                XmlFragment::deserialize(deserializer)
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut items = Vec::new();
                while let Some(item) = seq.next_element()? {
                    items.push(item);
                }
                Ok(XmlFragment::Seq(items))
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut entries = Vec::new();
                while let Some(key) = map.next_key::<String>()? {
                    // quick-xml reports `xsi:type` as `@type`; normalize it so the fragment can
                    // be written back with the namespace prefix intact.
                    let key = if key == "@type" {
                        "@xsi:type".to_string()
                    } else {
                        key
                    };
                    entries.push((key, map.next_value()?));
                }

                // A text-only element arrives as {"$text": "..."}; collapse it to a scalar.
                if entries.len() == 1 && entries[0].0 == "$text" {
                    return Ok(entries.remove(0).1);
                }

                Ok(XmlFragment::Map(entries))
            }
        }

        deserializer.deserialize_any(FragmentVisitor)
    }
}

impl Serialize for XmlFragment {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Map(entries) => {
                let mut map = serializer.serialize_map(Some(entries.len()))?;
                for (key, value) in entries {
                    map.serialize_entry(key, value)?;
                }
                map.end()
            }
            Self::Seq(items) => {
                let mut seq = serializer.serialize_seq(Some(items.len()))?;
                for item in items {
                    seq.serialize_element(item)?;
                }
                seq.end()
            }
            Self::Scalar(text) => serializer.serialize_str(text),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::XmlFragment;

    #[derive(Debug, PartialEq, ::serde::Serialize, ::serde::Deserialize)]
    struct Holder {
        #[serde(rename = "Data")]
        data: XmlFragment,
    }

    fn parse(xml: &str) -> Holder {
        quick_xml::de::from_str(xml).expect("parse")
    }

    fn render(holder: &Holder) -> String {
        quick_xml::se::to_string(holder).expect("serialize")
    }

    /// Parsing, rendering and parsing again must produce the same fragment.
    fn assert_stable(xml: &str) -> Holder {
        let first = parse(xml);
        let rendered = render(&first);
        let second = parse(&rendered);
        assert_eq!(first.data, second.data, "round-trip changed the fragment: {rendered}");
        first
    }

    #[test]
    fn captures_attributes_and_children() {
        let holder = assert_stable(
            r#"<Holder><Data xsi:type="SomeType"><Child A="1"><Leaf>text</Leaf></Child></Data></Holder>"#,
        );

        assert_eq!(holder.data.type_attribute(), Some("SomeType"));
        assert!(holder.data.get("Child").is_some());
        assert_eq!(
            holder.data.get("Child").and_then(|c| c.get("@A")).and_then(XmlFragment::as_str),
            Some("1")
        );
    }

    #[test]
    fn normalizes_namespace_stripped_type_attribute() {
        // quick-xml reports xsi:type as @type; the fragment stores @xsi:type.
        let holder = assert_stable(r#"<Holder><Data type="Plain"><X/></Data></Holder>"#);
        assert_eq!(holder.data.type_attribute(), Some("Plain"));

        let out = render(&holder);
        assert!(out.contains(r#"xsi:type="Plain""#), "{out}");
    }

    #[test]
    fn preserves_repeated_children() {
        let holder = assert_stable(r#"<Holder><Data><Item>1</Item><Item>2</Item></Data></Holder>"#);
        let out = render(&holder);
        assert_eq!(out.matches("<Item>").count(), 2, "{out}");
    }

    #[test]
    fn preserves_attributes_and_text_together() {
        let holder = assert_stable(r#"<Holder><Data A="1">text</Data></Holder>"#);
        assert_eq!(holder.data.get("@A").and_then(XmlFragment::as_str), Some("1"));
        assert_eq!(holder.data.get("$text").and_then(XmlFragment::as_str), Some("text"));
    }

    #[test]
    fn captures_scalar_content() {
        let holder = assert_stable("<Holder><Data>just text</Data></Holder>");
        assert_eq!(holder.data, XmlFragment::Scalar("just text".to_string()));
    }

    #[test]
    fn captures_empty_element() {
        let holder = assert_stable("<Holder><Data/></Holder>");
        let out = render(&holder);
        assert!(out.contains("<Data/>") || out.contains("<Data></Data>"), "{out}");
    }

    #[test]
    fn escapes_markup_in_text_and_attributes() {
        let holder = assert_stable(
            r#"<Holder><Data A="a&amp;b&quot;c&apos;d">x &lt; y &amp; z &gt; w</Data></Holder>"#,
        );

        assert_eq!(
            holder.data.get("@A").and_then(XmlFragment::as_str),
            Some("a&b\"c'd")
        );
        assert_eq!(
            holder.data.get("$text").and_then(XmlFragment::as_str),
            Some("x < y & z > w")
        );

        let out = render(&holder);
        assert!(out.contains("&amp;"), "{out}");
        assert!(out.contains("&quot;"), "{out}");
    }

    #[test]
    fn preserves_unicode() {
        let holder = assert_stable("<Holder><Data Name=\"héllo\">世界 🚀</Data></Holder>");
        assert_eq!(
            holder.data.get("@Name").and_then(XmlFragment::as_str),
            Some("héllo")
        );
    }

    #[test]
    fn preserves_deep_nesting() {
        let holder = assert_stable(
            "<Holder><Data><A><B><C><D value=\"1\">text</D></C></B></A></Data></Holder>",
        );
        let deep = holder
            .data
            .get("A")
            .and_then(|a| a.get("B"))
            .and_then(|b| b.get("C"))
            .and_then(|c| c.get("D"));
        assert_eq!(deep.and_then(|d| d.get("@value")).and_then(XmlFragment::as_str), Some("1"));
    }

    #[test]
    fn optional_element_can_be_absent() {
        #[derive(Debug, PartialEq, ::serde::Serialize, ::serde::Deserialize)]
        struct OptionalHolder {
            #[serde(rename = "Data")]
            data: Option<XmlFragment>,
        }

        let holder: OptionalHolder = quick_xml::de::from_str("<Holder/>").unwrap();
        assert_eq!(holder.data, None);
    }

    #[derive(Debug, PartialEq, ::serde::Serialize, ::serde::Deserialize)]
    #[serde(rename = "Payload")]
    struct Payload {
        #[serde(rename = "Count")]
        count: u32,
        #[serde(rename = "Name")]
        name: String,
    }

    #[test]
    fn converts_fragment_to_typed_value() {
        let holder = parse(
            r#"<Holder><Data><Count>3</Count><Name>Ada</Name></Data></Holder>"#,
        );

        let payload: Payload = holder.data.into_typed().expect("into_typed");
        assert_eq!(
            payload,
            Payload {
                count: 3,
                name: "Ada".to_string()
            }
        );
    }

    #[test]
    fn converts_typed_value_to_fragment_and_back() {
        let payload = Payload {
            count: 7,
            name: "Grace".to_string(),
        };

        let fragment = XmlFragment::from_typed(&payload, "SomeType").expect("from_typed");
        assert_eq!(fragment.type_attribute(), Some("SomeType"));

        let out = quick_xml::se::to_string(&Holder { data: fragment.clone() }).unwrap();
        assert!(out.contains(r#"xsi:type="SomeType""#), "{out}");
        assert!(out.contains("<Count>7</Count>"), "{out}");

        let restored: Payload = fragment.into_typed().expect("into_typed");
        assert_eq!(restored, payload);
    }

    #[test]
    fn from_typed_replaces_payload_type_attribute() {
        #[derive(Debug, ::serde::Serialize)]
        struct WithTypeAttribute {
            #[serde(rename = "@type")]
            kind: String,
            #[serde(rename = "@Name")]
            name: String,
        }

        let fragment = XmlFragment::from_typed(
            &WithTypeAttribute {
                kind: "Old".to_string(),
                name: "x".to_string(),
            },
            "New",
        )
        .unwrap();

        // The payload's own `type` attribute must not shadow the discriminator.
        assert_eq!(fragment.type_attribute(), Some("New"));
        assert_eq!(fragment.get("@type"), None);
        assert_eq!(fragment.get("@Name").and_then(XmlFragment::as_str), Some("x"));
    }

    #[test]
    fn cdata_is_captured_as_text() {
        let holder = assert_stable("<Holder><Data><![CDATA[x < y & z]]></Data></Holder>");
        assert_eq!(holder.data, XmlFragment::Scalar("x < y & z".to_string()));

        let out = render(&holder);
        assert!(out.contains("x &lt; y &amp; z"), "{out}");
        assert!(!out.contains("CDATA"), "{out}");
    }

    #[test]
    fn adjacent_cdata_sections_are_concatenated() {
        let holder = assert_stable("<Holder><Data><![CDATA[a]]><![CDATA[b]]></Data></Holder>");
        assert_eq!(holder.data, XmlFragment::Scalar("ab".to_string()));
    }

    #[test]
    fn namespace_prefixes_are_stripped_from_names() {
        let holder = assert_stable(
            r#"<Holder><Data xmlns:ns="urn:x"><ns:Child ns:attr="1"/></Data></Holder>"#,
        );

        // Declarations survive as attributes; element/attribute names lose the prefix.
        assert_eq!(
            holder.data.get("@xmlns:ns").and_then(XmlFragment::as_str),
            Some("urn:x")
        );
        let child = holder.data.get("Child").expect("prefix stripped from element");
        assert_eq!(child.get("@attr").and_then(XmlFragment::as_str), Some("1"));

        let out = render(&holder);
        assert!(out.contains(r#"xmlns:ns="urn:x""#), "{out}");
        assert!(out.contains(r#"<Child attr="1"/>"#), "{out}");
    }

    #[test]
    fn mixed_content_keeps_text_position() {
        let holder = assert_stable("<Holder><Data>before<B/>after</Data></Holder>");

        // serde round-trip
        let out = render(&holder);
        assert!(out.contains("before<B/>after"), "{out}");

        // renderer used by into_typed must not hoist text before children
        let rendered = holder.data.to_xml_string();
        assert_eq!(rendered, "<Value>before<B/>after</Value>");
    }

    #[test]
    fn comments_and_processing_instructions_are_dropped() {
        let holder = assert_stable("<Holder><Data><!-- comment --><X/><?pi x?></Data></Holder>");

        assert!(holder.data.get("X").is_some());
        let out = render(&holder);
        assert!(!out.contains("<!--"), "{out}");
        assert!(!out.contains("<?"), "{out}");
    }

    #[test]
    fn scalar_visitor_paths_map_to_text() {
        use serde::de::value::{
            BoolDeserializer, Error as ValueError, F64Deserializer, I64Deserializer,
            U64Deserializer, UnitDeserializer,
        };
        use serde::Deserialize as _;

        assert_eq!(
            XmlFragment::deserialize(I64Deserializer::<ValueError>::new(-5)).unwrap(),
            XmlFragment::Scalar("-5".to_string())
        );
        assert_eq!(
            XmlFragment::deserialize(U64Deserializer::<ValueError>::new(42)).unwrap(),
            XmlFragment::Scalar("42".to_string())
        );
        assert_eq!(
            XmlFragment::deserialize(F64Deserializer::<ValueError>::new(1.5)).unwrap(),
            XmlFragment::Scalar("1.5".to_string())
        );
        assert_eq!(
            XmlFragment::deserialize(BoolDeserializer::<ValueError>::new(true)).unwrap(),
            XmlFragment::Scalar("true".to_string())
        );
        assert_eq!(
            XmlFragment::deserialize(UnitDeserializer::<ValueError>::new()).unwrap(),
            XmlFragment::Scalar(String::new())
        );
    }}
