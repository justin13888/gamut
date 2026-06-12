//! Reconciliation between the legacy IIM and the modern XMP representations.

/// Reconciler between legacy IIM datasets and IPTC Photo Metadata (XMP).
///
/// The two carriers describe overlapping fields; an image may hold one, the other, or both with
/// differing values. This is the crate's **keystone**: applying the IPTC mapping guidelines'
/// precedence/sync rules to merge them into one coherent view, and to write both consistently.
/// Implementation pending (see issue #34).
pub struct IimXmpReconciler;
