fn main() {
    // Use the embedded SVG from the barbs module
    let svg_data = include_str!("../assets/wind-barbs/10.svg");

    println!("SVG content length: {} bytes", svg_data.len());
    println!("First 100 chars: {}", &svg_data[..100.min(svg_data.len())]);

    let opt = usvg::Options::default();
    let tree = match usvg::Tree::from_str(svg_data, &opt) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Failed to parse SVG: {}", e);
            return;
        }
    };

    println!("SVG parsed successfully");
    println!("Tree size: {:?}", tree.size());

    let mut pixmap = tiny_skia::Pixmap::new(250, 250).unwrap();
    println!("Pixmap created: {}x{}", pixmap.width(), pixmap.height());

    let transform = tiny_skia::Transform::identity();
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    println!("Render complete");

    // Check for non-transparent pixels
    let data = pixmap.data();
    let mut non_zero = 0;
    for i in (0..data.len()).step_by(4) {
        if data[i + 3] > 0 {
            non_zero += 1;
        }
    }
    println!(
        "Non-transparent pixels: {}/{}",
        non_zero,
        pixmap.width() * pixmap.height()
    );
}
