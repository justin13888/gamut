//! `gamut-xmp` — XMP (Extensible Metadata Platform) parsing and canonical serialization.
//!
//! XMP is the RDF/XML metadata packet embedded in images (the WebP `XMP ` chunk, the AVIF/HEIF
//! `mime` item of type `application/rdf+xml`, a JPEG `APP1` segment), wrapped in an `<?xpacket?>`
//! processing instruction. This crate parses such a packet into the [`XmpMeta`] property graph —
//! simple, structured, and `Bag`/`Seq`/`Alt` array values, with qualifiers and language
//! alternatives — and serializes a graph back to **canonical RDF/XML** (Part 1 §7), which fixes the
//! element-vs-attribute encoding, namespace placement, and array/struct nesting so output is
//! stable, diffable, and round-trippable.
//!
//! Implemented from the **Adobe XMP Specification, Parts 1–3** (equivalent to ISO 16684-1/-2;
//! `references/xmp`).
//!
//! # Quick start
//!
//! [`XmpMeta::from_packet`] reads a packet; [`XmpMeta::to_packet`] writes one. Accessors like
//! [`XmpMeta::get_text`] / [`XmpMeta::set_text`] and [`XmpMeta::get_lang_alt`] /
//! [`XmpMeta::set_lang_alt`] cover the common cases; [`WellKnownNs`] supplies the standard schema
//! URIs so you do not hand-write them.
//!
//! # Design notes
//!
//! - **Reads more than it writes.** The parser accepts the broad RDF/XML input XMP permits (Part 1
//!   Annex C / §7.9 — attribute or element form, `rdf:parseType="Resource"`, abbreviations); the
//!   serializer emits one fixed canonical form.
//! - **UTF-8.** Packets are read and written as UTF-8 (a leading byte-order mark is tolerated on
//!   read but not emitted). Part 1 §7.1 also allows UTF-16/32, which are reported as unsupported.
//! - **`quick-xml` is internal.** The XML lexer is an implementation detail and does not appear in
//!   the public API (errors are surfaced via [`XmpError`]), so it can be changed without a breaking
//!   change.
//! - **Memory-safe on hostile input.** `#![forbid(unsafe_code)]` — XMP is XML from untrusted files.
#![forbid(unsafe_code)]

pub mod error;
pub mod model;
pub mod namespace;
pub mod packet;
pub mod reader;
pub mod writer;

pub use error::{Result, XmpError};
pub use model::{XmpArray, XmpItem, XmpMeta, XmpProperty, XmpValue};
pub use namespace::{Namespace, WellKnownNs};
pub use packet::XmpPacket;
pub use reader::XmpReader;
pub use writer::XmpWriter;
