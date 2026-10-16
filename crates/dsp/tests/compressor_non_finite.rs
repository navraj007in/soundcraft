//! A non-finite sample into the compressor must not reach its output, now or `lookahead` samples later.

use soundcraft_dsp::create;

#[test]
fn a_nan_or_infinity_never_comes_out_of_the_lookahead_delay() {
    let mut c = create("compressor").expect("built-in");
    c.prepare(48_000.0, 512, 1);
    assert!(c.set_param("lookahead", 5.0));
    let mut io = vec![vec![0.25f32; 512]];
    io[0][10] = f32::NAN;
    io[0][11] = f32::INFINITY;
    io[0][12] = f32::NEG_INFINITY;
    c.process(&mut io, 512);
    assert!(io[0].iter().all(|x| x.is_finite()), "block with the bad samples");
    // The delayed copy of those samples arrives in later blocks: still finite.
    for _ in 0..4 {
        let mut next = vec![vec![0.25f32; 512]];
        c.process(&mut next, 512);
        assert!(next[0].iter().all(|x| x.is_finite()));
    }
}
