//! `gamut-xmp` — XMP (Extensible Metadata Platform) metadata parsing and serialization.
//!
//! XMP is the RDF/XML metadata packet embedded in images (the WebP `XMP ` chunk, the AVIF/HEIF
//! `mime` item of type `application/rdf+xml`, a JPEG `APP1` segment) and wrapped in an
//! `<?xpacket?>` processing instruction. This crate models the XMP property graph — simple,
//! structured, and `Bag`/`Seq`/`Alt` array values, with qualifiers and language alternatives — and
//! the canonical RDF/XML serialization.
//!
//! Structure follows the **Adobe XMP Specification, Parts 1–3** (equivalent to ISO 16684-1/-2;
//! `references/xmp`). XMP uses a constrained RDF/XML subset, so the parser does not need a
//! general-purpose XML engine.
//!
//! > **Open decision (XML reader):** whether that subset is parsed by a hand-rolled reader (the
//! > default, keeping the workspace dependency-light) or a vetted crate (`quick-xml`) is recorded
//! > in `STATUS.md` and settled before the P2 implementation phase. Either way the public surface
//! > below is unaffected.
//!
//! Placeholder skeleton — implementation pending (see issue #34). The type declarations below
//! sketch the data model; no parsing/serialization exists yet.
#![forbid(unsafe_code)]

pub mod model;
pub mod namespace;
pub mod packet;
pub mod reader;
pub mod writer;

pub use model::{XmpArray, XmpMeta, XmpProperty, XmpValue};
pub use namespace::{Namespace, WellKnownNs};
pub use packet::XmpPacket;
pub use reader::XmpReader;
pub use writer::XmpWriter;
