fn main() {
    let svg_data = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 250 250">
    <circle cx="125" cy="125" r="20" fill="red"/>
</svg>"##;

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
        if data[i+3] > 0 {
            non_zero += 1;
        }
    }
    println!("Non-transparent pixels: {}/{}", non_zero, pixmap.width() * pixmap.height());
}
