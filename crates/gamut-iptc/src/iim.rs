//! The legacy IPTC-IIM record/dataset model.

/// One IIM dataset: a (record, dataset) tag and its raw value (IPTC-IIM 4.2 §3).
///
/// IIM is a flat stream of datasets, each introduced by a `0x1C` marker, a 1-byte record number, a
/// 1-byte dataset number, a length, and the value. A dataset may repeat (e.g. multiple keywords).
pub struct IimDataSet {
    /// The record number this dataset belongs to (see [`IimRecord`]).
    pub record: u8,
    /// The dataset number within the record (see [`IimTag`] for record 2).
    pub dataset: u8,
    /// The raw dataset value (decoded into typed/UTF-8 form during implementation).
    pub data: Vec<u8>,
}

/// The IIM record a dataset lives in (IPTC-IIM 4.2). The Application record (2) carries the
/// photo-description fields; the others are envelope/administrative. Representative subset.
pub enum IimRecord {
    /// Record 1 — the Envelope record (routing/format metadata).
    Envelope,
    /// Record 2 — the Application record (the descriptive photo metadata).
    Application,
    /// Record 7 — the Pre-ObjectData Descriptor record.
    PreObjectData,
    /// Record 8 — the ObjectData record.
    ObjectData,
    /// Record 9 — the Post-ObjectData Descriptor record.
    PostObjectData,
}

/// A dataset in the Application record (record 2) — the descriptive IPTC fields. Representative
/// subset; the full set is filled in during implementation. Each maps to a dataset number.
pub enum IimTag {
    /// `2:25` Keywords (repeatable).
    Keywords,
    /// `2:120` Caption/Abstract.
    Caption,
    /// `2:80` By-line (creator).
    Byline,
    /// `2:105` Headline.
    Headline,
    /// `2:90` City.
    City,
    /// `2:101` Country/Primary Location Name.
    Country,
    /// `2:55` Date Created.
    DateCreated,
    /// `2:116` Copyright Notice.
    CopyrightNotice,
}
