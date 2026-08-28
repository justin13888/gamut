//! Per-plane sample geometry (AV1 §5.5.2 `subsampling_x`/`subsampling_y`, §5.11.34 plane bases).
//!
//! Every subsampling shift in the encoder lives here. That is deliberate: at 4:4:4 both shifts are
//! zero, so an inline `>> ss_x` written at a call site is indistinguishable from the identity and
//! its mutants are unkillable. Behind [`PlaneGeom`] the same arithmetic is unit-testable at
//! `ss = 1`, which is the only way the mutation gate can see it before the subsampled coding path
//! lands (issues #390 / #391).

use gamut_color::ChromaSubsampling;

/// The sample geometry of one coded plane.
///
/// Two dimensions, not one, and they round differently — this is the subtlety the whole module
/// exists to hold:
///
/// - **Visible** (`w`, `h`) is the source/display extent, `ceil(luma / (1 << ss))`. Ceiling,
///   because an odd luma dimension keeps its half-covering edge chroma sample.
/// - **Coded** (`coded_w`, `coded_h`) is the padded MI-grid extent that `recon` is allocated at and
///   strided by. `mi_cols`/`mi_rows` are always even (`2 * ((n + 7) >> 3)`), so the coded extent is
///   a multiple of 8 luma samples and halving it is **exact** — a plain `>>`, with nothing to round.
///
/// Using one rounding rule for both is wrong in both directions, so they are computed separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PlaneGeom {
    /// Visible plane width in samples.
    pub w: usize,
    /// Visible plane height in samples.
    pub h: usize,
    /// Coded plane width — the row stride of this plane's reconstruction buffer.
    pub coded_w: usize,
    /// Coded plane height.
    pub coded_h: usize,
    /// `subsampling_x` for this plane (always 0 for luma).
    pub ss_x: u32,
    /// `subsampling_y` for this plane (always 0 for luma).
    pub ss_y: u32,
}

impl PlaneGeom {
    /// The three plane geometries of a frame, `[luma, u, v]`.
    ///
    /// `width`/`height` are the luma display dimensions and `mi_cols`/`mi_rows` the luma MI grid.
    /// Luma always has `ss_x = ss_y = 0`; the chroma planes take their shifts from `subsampling`.
    pub(crate) fn frame(
        width: usize,
        height: usize,
        mi_cols: usize,
        mi_rows: usize,
        subsampling: ChromaSubsampling,
    ) -> [PlaneGeom; 3] {
        let (sx, sy) = subsampling.subsampling();
        let (sx, sy) = (u32::from(sx), u32::from(sy));
        let plane = |ss_x: u32, ss_y: u32| PlaneGeom {
            // Ceiling on the visible axes: a 5-sample-wide luma plane has 3 chroma samples.
            w: width.div_ceil(1 << ss_x),
            h: height.div_ceil(1 << ss_y),
            // Exact on the coded axes: `mi_cols * 4` is a multiple of 8.
            coded_w: (mi_cols * 4) >> ss_x,
            coded_h: (mi_rows * 4) >> ss_y,
            ss_x,
            ss_y,
        };
        [plane(0, 0), plane(sx, sy), plane(sx, sy)]
    }

    /// Number of samples in this plane's reconstruction buffer.
    pub(crate) fn len(self) -> usize {
        self.coded_w * self.coded_h
    }

    /// The luma MI column covering this plane's sample column `x` — `(x << ss_x) >> 2`.
    ///
    /// The MI grid is defined on luma, so a chroma coordinate is scaled up before it is divided
    /// into 4-sample cells. At `ss_x = 0` this is the plain `x >> 2`.
    pub(crate) fn mi_col(self, x: usize) -> usize {
        (x << self.ss_x) >> 2
    }

    /// The luma MI row covering this plane's sample row `y` — `(y << ss_y) >> 2`.
    pub(crate) fn mi_row(self, y: usize) -> usize {
        (y << self.ss_y) >> 2
    }

    /// Maps a **luma** sample position into this plane's own coordinates (`x >> ss_x`,
    /// `y >> ss_y`).
    ///
    /// A method rather than an inline shift on purpose: at 4:4:4 both shifts are zero, so
    /// `>>` and `<<` are indistinguishable at every call site the encoder can currently reach.
    /// Here the direction is pinned by a test at `ss = 1`.
    pub(crate) fn scale_pos(self, x: usize, y: usize) -> (usize, usize) {
        (x >> self.ss_x, y >> self.ss_y)
    }

    /// Maps a **luma** block extent into this plane's own extent (`w >> ss_x`, `h >> ss_y`) — the
    /// §7.15.1 chroma CDEF block, for instance, is `(8 >> ss_x) x (8 >> ss_y)`.
    ///
    /// Separate from [`scale_pos`](Self::scale_pos) despite the identical arithmetic: a position
    /// and an extent are different quantities, and under formats gamut does not yet emit they stop
    /// agreeing (a 1-sample extent cannot halve).
    pub(crate) fn scale_extent(self, w: usize, h: usize) -> (usize, usize) {
        (w >> self.ss_x, h >> self.ss_y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `mi_cols`/`mi_rows` as `FrameEncoder::new` derives them, so the table below is stated in the
    /// same terms the encoder uses.
    fn mi(width: usize, height: usize) -> (usize, usize) {
        (2 * ((width + 7) >> 3), 2 * ((height + 7) >> 3))
    }

    #[test]
    fn luma_is_never_subsampled() {
        for ss in [
            ChromaSubsampling::Cs444,
            ChromaSubsampling::Cs422,
            ChromaSubsampling::Cs420,
        ] {
            let (mc, mr) = mi(17, 13);
            let g = PlaneGeom::frame(17, 13, mc, mr, ss)[0];
            assert_eq!((g.ss_x, g.ss_y), (0, 0), "{ss:?}");
            assert_eq!((g.w, g.h), (17, 13), "{ss:?}");
            assert_eq!((g.coded_w, g.coded_h), (24, 16), "{ss:?}");
        }
    }

    #[test]
    fn visible_rounds_up_while_coded_halves_exactly() {
        // The load-bearing asymmetry. 17x13 luma ⇒ mi 6x4 ⇒ coded 24x16.
        //   4:2:0 chroma: visible ceil(17/2) x ceil(13/2) = 9x7, coded 24/2 x 16/2 = 12x8.
        //   4:2:2 chroma: visible 9x13, coded 12x16.
        // A mutant that used `>>` for the visible axis would give 8x6; one that used `div_ceil` for
        // the coded axis would agree here but not at an odd MI count, which cannot occur — so the
        // literal coded values are pinned rather than re-derived.
        let (mc, mr) = mi(17, 13);
        let g420 = PlaneGeom::frame(17, 13, mc, mr, ChromaSubsampling::Cs420)[1];
        assert_eq!((g420.w, g420.h), (9, 7));
        assert_eq!((g420.coded_w, g420.coded_h), (12, 8));
        assert_eq!((g420.ss_x, g420.ss_y), (1, 1));

        let g422 = PlaneGeom::frame(17, 13, mc, mr, ChromaSubsampling::Cs422)[1];
        assert_eq!((g422.w, g422.h), (9, 13));
        assert_eq!((g422.coded_w, g422.coded_h), (12, 16));
        assert_eq!((g422.ss_x, g422.ss_y), (1, 0));

        // The smallest frame: one luma sample still has one chroma sample, on an 8x8 coded grid.
        let (mc, mr) = mi(1, 1);
        let tiny = PlaneGeom::frame(1, 1, mc, mr, ChromaSubsampling::Cs420)[1];
        assert_eq!((tiny.w, tiny.h), (1, 1));
        assert_eq!((tiny.coded_w, tiny.coded_h), (4, 4));

        // 4:4:4 leaves both extents alone, and both chroma planes share one geometry.
        let (mc, mr) = mi(64, 64);
        let g = PlaneGeom::frame(64, 64, mc, mr, ChromaSubsampling::Cs444);
        assert_eq!(g[1], g[2]);
        assert_eq!((g[1].w, g[1].coded_w), (64, 64));
    }

    #[test]
    fn coded_extent_stays_a_whole_number_of_mi_cells() {
        // Invariant the exact `>>` relies on: the coded extent is a multiple of 4 for every plane
        // and every layout, and never smaller than the visible extent.
        for w in 1..=17usize {
            for h in 1..=17usize {
                let (mc, mr) = mi(w, h);
                for ss in [
                    ChromaSubsampling::Cs444,
                    ChromaSubsampling::Cs422,
                    ChromaSubsampling::Cs420,
                ] {
                    for g in PlaneGeom::frame(w, h, mc, mr, ss) {
                        assert_eq!(g.coded_w % 4, 0, "{w}x{h} {ss:?}");
                        assert_eq!(g.coded_h % 4, 0, "{w}x{h} {ss:?}");
                        assert!(g.w <= g.coded_w, "{w}x{h} {ss:?}");
                        assert!(g.h <= g.coded_h, "{w}x{h} {ss:?}");
                        assert_eq!(g.len(), g.coded_w * g.coded_h);
                    }
                }
            }
        }
    }

    #[test]
    fn luma_positions_and_extents_scale_down_into_a_plane() {
        let (mc, mr) = mi(64, 64);
        let g = PlaneGeom::frame(64, 64, mc, mr, ChromaSubsampling::Cs420);
        // Luma is the identity in both directions.
        assert_eq!(g[0].scale_pos(16, 24), (16, 24));
        assert_eq!(g[0].scale_extent(8, 8), (8, 8));
        // 4:2:0 halves both axes. Asserting the halved values (not just "not equal") is what
        // distinguishes `>>` from `<<`, which are the same operation at 4:4:4.
        assert_eq!(g[1].scale_pos(16, 24), (8, 12));
        assert_eq!(g[1].scale_extent(8, 8), (4, 4));
        // 4:2:2 halves x only, so a transposed shift is visible here and nowhere else.
        let g422 = PlaneGeom::frame(64, 64, mc, mr, ChromaSubsampling::Cs422);
        assert_eq!(g422[1].scale_pos(16, 24), (8, 24));
        assert_eq!(g422[1].scale_extent(8, 8), (4, 8));
    }

    #[test]
    fn mi_lookup_scales_a_chroma_coordinate_back_to_the_luma_grid() {
        let (mc, mr) = mi(64, 64);
        let luma = PlaneGeom::frame(64, 64, mc, mr, ChromaSubsampling::Cs420)[0];
        let chroma = PlaneGeom::frame(64, 64, mc, mr, ChromaSubsampling::Cs420)[1];
        // Luma: the plain 4-sample cell division.
        assert_eq!(luma.mi_col(0), 0);
        assert_eq!(luma.mi_col(3), 0);
        assert_eq!(luma.mi_col(4), 1);
        assert_eq!(luma.mi_row(9), 2);
        // Chroma at ss = 1: sample 3 sits at luma 6, i.e. MI cell 1 — the case that distinguishes
        // `(x << ss) >> 2` from a bare `x >> 2`, and the reason this lives behind a function.
        assert_eq!(chroma.mi_col(0), 0);
        assert_eq!(chroma.mi_col(1), 0);
        assert_eq!(chroma.mi_col(2), 1);
        assert_eq!(chroma.mi_col(3), 1);
        assert_eq!(chroma.mi_row(2), 1);
        // 4:2:2 subsamples x only, so its rows track luma while its columns do not.
        let c422 = PlaneGeom::frame(64, 64, mc, mr, ChromaSubsampling::Cs422)[1];
        assert_eq!(c422.mi_col(2), 1);
        assert_eq!(c422.mi_row(2), 0);
    }
}
