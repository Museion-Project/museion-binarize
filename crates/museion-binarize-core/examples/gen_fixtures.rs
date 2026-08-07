// Development helper: write the synthetic fixtures to a directory.
use museion_binarize_core::test_fixtures as f;
fn main() {
    let dir = std::env::args().nth(1).expect("usage: gen_fixtures <dir>");
    let d = std::path::Path::new(&dir);
    for (name, bytes) in [
        ("orientation.pdf", f::orientation_and_polarity()),
        ("mixed.pdf", f::mixed_page_sizes()),
        ("rotations.pdf", f::page_rotations()),
        ("threshold.pdf", f::threshold_patterns()),
    ] {
        std::fs::write(d.join(name), &bytes).expect("write");
        println!("{name}: {} bytes", bytes.len());
    }
}
