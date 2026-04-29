// Tests for the pure-Nexus PCG PRNG (nexus-dvr6.9.6).
//
// The fixture seeds state via the test-only `seed_with` hatch in
// `nxlib/stdlib/rand.nx` and prints the first 10 PCG outputs for seed=1
// followed by one output for seed=2. Reference values are computed
// offline as `state'.wrapping_mul(MUL).wrapping_add(INC) & 0x7FFFFFFFFFFFFFFF`
// over u64 with state₀ = seed.
//
// Failure of any line catches PCG step regressions (constants drifted,
// multiply/add order swapped) AND state-cell layout corruption
// (load/store offset misalignment).

use crate::harness::exec_nxc_core_capture_stdout;

#[test]
fn rand_determinism_pcg_step_byte_equal_across_runs() {
    let out = exec_nxc_core_capture_stdout("bootstrap/tests/fixtures/nxc/test_rand_determinism.nx");
    let lines: Vec<&str> = out.lines().map(str::trim).collect();
    let expected = [
        // Phase A: seed = 1, ten next_i64() outputs.
        "7806831264735756412",
        "173536691264035611",
        "2736747771374053902",
        "7062582979898595269",
        "5450049017633417712",
        "9431502868738175",
        "994931806658971810",
        "1206773305466921929",
        "6266840599827907236",
        "3660572683296592931",
        // Phase B: reset + seed = 2, one next_i64() output.
        // Must differ from r1 above — proves the smoke contract that
        // independent seeds produce independent sequences.
        "4947595451727773609",
    ];
    assert_eq!(
        lines.len(),
        expected.len(),
        "expected {} lines, got {} — full stdout:\n{}",
        expected.len(),
        lines.len(),
        out
    );
    for (i, (got, want)) in lines.iter().zip(expected.iter()).enumerate() {
        assert_eq!(
            got, want,
            "line {} mismatch: got {:?}, want {:?} — full stdout:\n{}",
            i + 1,
            got,
            want,
            out
        );
    }
    // Smoke: phase B's first output must differ from phase A's first.
    // Caught by the per-line assertion above, but called out here so the
    // smoke contract is explicit at the test boundary.
    assert_ne!(
        lines[0], lines[10],
        "seed=1 and seed=2 produced identical first output — PRNG is seed-insensitive"
    );
}
